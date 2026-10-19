# ClipCut — instructions for agents

ClipCut is a Windows-first desktop video editor with a CapCut-style interface, built on a fork of
FilmCraft (a clean-room, pure-Rust editing engine; upstream: https://github.com/storytold/filmcraft,
remote `upstream`). Read `README.md` for what it is and the roadmap.

## Where things live
- `crates/*` except `clipcut-*`: FilmCraft's engine. **Change it as little as possible** so upstream
  improvements keep merging (`git fetch upstream && git merge upstream/main`). When the engine needs a
  fix, make it small and self-contained, ideally something upstream would accept.
- `crates/clipcut-core` (layer 5): the engine operations the app uses (open a clip, preview frames,
  playback audio, export jobs), all through engine commands (`Session::execute`). No UI toolkit here.
  Tested against clips ffmpeg generates (`cargo test -p clipcut-core`).
- `apps/clipcut` (layer 6): the Tauri app. `src/main.rs` holds the commands the interface invokes,
  `src/playback.rs` the audio output (the sound card is the playback clock), `ui/` the interface in
  plain HTML/CSS/JS with **no build step** (`window.__TAURI__` globals). Engine work stays in Rust; the
  interface only asks for it. `--self-test <in> <out> <report.json>` runs open/preview/sound/export
  without a window (CI runs it on Windows).
- Clip Studio (https://github.com/portdel18/clip-stu) is the Graphics tab: a copy in
  `apps/clipcut/ui/graphics/`, never edited here. Change Clip Studio in its own repo, then run
  `python3 packaging/clipcut/sync-graphics.py ../clip-stu` (after `npm ci` there): it copies the page
  and bundles its pinned libraries locally, because the CSP allows scripts only from the app. The
  page talks to ClipCut through `window.__clipcutHost` (`ui/app.js`) and `CS.hosted` (its
  `js/core.js`, "Inside ClipCut"); frames go to `src/graphics.rs` as raw request bodies, and
  `clipcut-core/src/graphics.rs` imports them as an image sequence.
- `.github/workflows/clipcut-windows.yml`: tests, the NSIS installer, the self-test, the artifact.
  `.github/workflows/clipcut-release.yml` publishes a release (tag `v*` or a version typed in
  Actions): code signing and update signing switch on with the repository's secrets and variables
  (`packaging/clipcut/RELEASING.md`). The updater (`src/updates.rs`) is registered only when
  `CLIPCUT_UPDATER_PUBKEY` was set at build time; without it, `app.updater()` would panic, so check
  `updates::key()` first. Its manifest is `latest.json` on the latest release. Without a key, Windows
  builds update straight from GitHub's API (`releases/latest`): the `-setup.exe` asset, refused unless
  its size and the SHA-256 `digest` GitHub lists match, run with `/P /UPDATE /R`.

## Running and looking at the app
- Linux needs the Tauri system libraries (see README). Under a headless session:
  `xvfb-run -s "-screen 0 1600x1000x24" target/debug/clipcut clip.mp4` and screenshot with
  `import -window root shot.png`. **Look at the result** after interface changes.
- The user tests on Windows with real footage; CI is the only Windows build here.
- The `ui/` files are built into the binary: rebuild (`cargo build -p clipcut`) after changing them.
  `clipcut a.mp4 b.mp4` starts a project with those clips (handy under Xvfb, where the file dialog is
  hard to drive). WebKit (Linux, and Safari-like engines) needs `-webkit-user-select`, or drags paint
  the selection highlight over canvases.
- The timeline UI redraws from the `TimelineView` every edit returns; edits run one at a time
  (`serial` in `app.js`). Each `Studio` edit is one undo step (`group` in `clipcut-core`'s
  `timeline.rs`): keep it that way for new edits.
- Edits stop playback (the player plays a snapshot of the sequence), except mixer changes:
  `set_mixer` hands the playing `Player` a new feed (`Player::update`) so faders are heard as they
  move. Meters come from the engine's `LiveMix` in each feed (`AudioFeed::meters`); without a sound
  device the player still mixes along the wall clock so they move.
- Captions (`clipcut-core/src/captions.rs`) follow the main track's speech: every caption's cue id
  holds its media anchor (`cc:<item>:<from>:<to>`), and main-track edits go through `group_follow`,
  which re-places them afterwards (and puts the main track's transitions back on their cuts,
  `transitions.rs`). A new edit that moves main-track clips must use `group_follow`.
  Whisper models can't be downloaded from this sandbox (Hugging Face is blocked): test with
  `Studio::hear` / `CLIPCUT_TEST_TRANSCRIPT`; the Windows CI step runs the real model on real speech.
- Text animations (`clipcut-core/src/animations.rs`) are keyframes on the text layer kept on the
  clip's ends by the engine's Responsive Design – Time; which ones a clip has lives in a clip marker
  named "ClipCut animation". Any edit of an animated text's words or style must go through
  `hold_animations` / `restore_animations` (as `apply_props` does) so they are rebuilt for it.
- Beats are clip markers named "Beat" (media time, so they move and trim with the clip), found by
  `filmcraft_audio_dsp::remix::analyze`. Export goes through `export_as` with a choice from
  `export_choices`; `export(path)` stays for the self-test.
- Titles are the engine's graphics templates (`graphics.template.*`); a clip made from one has a
  template link (`ClipView.title`) and is edited through its controls, never the text keys
  (its first layer may be a shape). The user's own titles live in "Graphics Templates" beside the
  engine's preferences file, so `restore` calls `Studio::set_data_dir` with the app data folder.
- The player's frames come from `FrameFeed` (`clipcut-core/src/preview.rs`: a project snapshot
  that renders on any thread at a scale snapped to 1, ½, ¼, ⅛ by `preview_scale`). While playing,
  `apps/clipcut/src/ahead.rs` renders ahead on 2–3 workers and `frame` serves from it without the
  studio lock. Proxies are the engine's (`media.createProxies`); `Studio::poll` attaches finished
  ones, and the Media panel calls it when the job ends.
- Graphics follow the video: opening Graphics calls `CS.hosted.setBackdrop` with the frame at the
  playhead (`graphic_backdrop`, which leaves out the graphic being changed), and after a Ratio
  change `offerRemake` re-renders graphics in the old shape through `CS.hosted.add` one by one
  (`GR.batch` keeps `finish` from closing Graphics between them).
- Drops: the window takes every file drop itself (`dragDropEnabled`; on Windows WebView2's drop handler is
  replaced, so HTML5 drag and drop never reaches the page). In-page dragging uses pointer events, never
  `draggable`. While Graphics is open, dropped files are read back through `graphics::dropped_file` (only
  the latest drop's paths) and handed to Clip Studio's drop zone under the pointer with `CS.hosted.drop`.
- Media: `Studio::new` sets the engine's `media.defaultMediaScaling` to `setToFrameSize`, so a clip of
  another size fits the frame through its Scale when added. Footage the engine can't decode (H.264
  beyond 8-bit 4:2:0 progressive, codecs without a decoder) shows the unreadable slate:
  `Studio::check_picture` names it in plain words on import (`MediaView.unreadable`). Folder and file
  names from the user go through `safe_name` (Windows device names, trailing dots).
- Disk is tight here: `rm -rf target/debug/incremental` when a build fails with "no space left".
- Interface copy: plain words from the user's side, errors that say what went wrong and how to fix it.
  One dark theme from the tokens at the top of `ui/app.css`; amber (`--accent`) marks the active thing.
- Branding: ClipCut's own name and icon (`packaging/clipcut/icon.py`). Never ArtCraft/FilmCraft
  branding in the app, and never CapCut's (or anyone's) artwork: "CapCut-style" means layout and feel.

## FilmCraft's own workflow (for engine work)
`plan/` is maintainer-local in FilmCraft and absent here; use the public docs: [`ROADMAP.md`](ROADMAP.md),
[`docs/gaps.md`](docs/gaps.md), [`docs/architecture.md`](docs/architecture.md),
[`docs/contributing.md`](docs/contributing.md), [`docs/testing.md`](docs/testing.md).

## Non-negotiables
**Read [`AGENTS.md`](AGENTS.md) first; its rules override everything here.**
- **Never crash** ([`AGENTS.md`](AGENTS.md) §0, standard: [`craftrules/standards/never-crash.md`](https://github.com/storytold/craftrules/blob/main/standards/never-crash.md)). People depend on FilmCraft; a crash loses their work, so this outranks feature work. No `unwrap`/`expect`/`panic!`/`unreachable!`/`todo!`/`unimplemented!` or `unsafe` outside tests: return `Result<T, E>` and propagate with `?` (`ok_or`, `let … else`, `if let`). Treat every number from media, project files, presets or CLI/MCP/control params as hostile: `get()` instead of `[i]`, checked/saturating math, no division by possibly-zero values, capped allocations, bounded recursion. `lock().unwrap_or_else(PoisonError::into_inner)`; background jobs run under `catch_unwind`. The panic hook + UI `catch_unwind` (`crates/ui-egui/src/crash.rs`) are the last-resort guard, not a licence. Every crash fix gets a regression test that panicked before. In particular (§1): never use Adobe iconography, images or any other Adobe asset. Every asset must be openly licensed (OSS / public domain / Creative Commons, or original work by a contributor) and must have a `<file>.attribution` sidecar plus an entry in `ATTRIBUTION.md`. `cargo xtask assets` enforces this.
- **Fonts** live in [storytold/craft-fonts](https://github.com/storytold/craft-fonts), never in this repo (no new font files here; [`AGENTS.md`](AGENTS.md) §1.9, standard: [`craftrules/standards/fonts.md`](https://github.com/storytold/craftrules/blob/main/standards/fonts.md)). Build with them: `git clone https://github.com/storytold/craft-fonts ../craft-fonts && CRAFT_FONTS_DIR="$PWD/../craft-fonts" cargo run -p filmcraft` (`crates/text/build.rs` embeds them as `filmcraft_text::fonts::CRAFT_FONTS`, used for Japanese in the UI and titles). Unset, `CRAFT_FONTS` is empty and everything must still build, test and run; never add craft-fonts to a `Cargo.toml`. Tests on its glyphs skip when it is empty, so run the gates with and without `CRAFT_FONTS_DIR` when you touch fonts.
- **Clean-room.** Never read/disassemble anything inside Adobe app bundles (names/listings only). Never copy Adobe icons, presets, LUTs, fonts. Never copy GPL/LGPL/AGPL code (FFmpeg, x264, x265, MLT, Kdenlive, Shotcut, Olive, LAME…). ffmpeg/ffprobe run only as external test oracles / fixture generators — never linked or shipped.
- **Pure Rust** in the engine (ClipCut's interface in `apps/clipcut/ui` is the one exception: HTML/JS shown by the system WebView through Tauri). OS media APIs only via Rust bindings in `crates/platform` (hardware decoding; the one crate allowed `unsafe`, see `AGENTS.md` §0.3 and `docs/adr/0001-platform-ffi.md`), optional, behind traits, with our decoders as the fallback.
- **Layering** ([`docs/architecture.md`](docs/architecture.md) §1, enforced by `cargo xtask layers`): nothing below L5 depends on egui/eframe/winit/rfd/cpal. L0 codec/container crates depend only on `filmcraft-bitstream`.
- **Exact time:** all time is `filmcraft_time::Tick` (254 016 000 000/s). Never use f64 seconds for edit math.
- **Everything is a command** (`crates/engine`): id, label, menu path, shortcut, params, enabled(), run(). UI, CLI, control channel and MCP all dispatch by id.
- **Everything is agent-drivable:** every interactive widget registers an automation id; UI state is serde so the control channel can read/set it.
- **Quality gates** before every commit (ClipCut: at least fmt, clippy and tests on the crates you touched, `cargo xtask layers` and `cargo xtask assets`): `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo xtask layers`, `cargo xtask assets`, `cargo xtask wasm` (checks L0–L4 crates). `cargo xtask ci` runs them all.
- **Commits:** one task id per commit (`M2.4: CABAC residual decoding`). Only green states. End messages with the attribution line required by the environment.

## Running FilmCraft's own app (engine work)
- `cargo run -p filmcraft -- --control 9876` opens the desktop app with the JSON-lines control server (see `docs/control-protocol.md`).
- For UI work, **look at the result**: drive via the control channel and take `ui.screenshot`, compare with `plan/premiere/screenshots/` (maintainer-local; see [`docs/agents.md`](docs/agents.md) §3).
- Parallel agents: separate git worktrees and `CARGO_TARGET_DIR=target/agent-<name>`; keep every `Cargo.toml` valid at all times (the `crates/*` glob means one broken manifest breaks everyone).
- Test fixtures: generate with ffmpeg into `target/fixtures/` (never commit media). See [`docs/testing.md`](docs/testing.md).
