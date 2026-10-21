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
  its size and the SHA-256 `digest` GitHub lists match, run with `/P /UPDATE /R`. Windows CI updates
  an installed copy to a newer build end to end (`CLIPCUT_UPDATE_TEST`, a local feed; keep it green).

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
- The timeline takes 44% of the window by default (`--tl-h` on `.app`); the divider above it
  (`#tl-split`) resizes it and remembers the share (`clipcut.tlHeight`). `layoutRows` centres the
  lanes in it (`S.rowShift`), so code that places things by lane uses `S.rows`, never `ROW` sums.
- Lanes: `Studio::move_clip` drops a clip on the lane asked for, or on a new lane right beside it
  when that is taken there (`lane_beside`); pictures (not text) also move off the main track onto
  a lane (`move_off_main`: its sound goes to a sound lane, still linked, the main track closes up)
  and into it at the nearest cut (`move_into_main`, via `pack`). In the interface, a main clip
  dragged above the main track lifts onto a lane (`reorderDrag`), and `laneFor` offers the main
  track to lane pictures.
- Linkage (`linkage.rs`, on by default, `#link-btn` / `Studio::set_linkage`, remembered as
  `clipcut.linkage`): before every `group_follow` edit each lane clip is tied to the main-track clip
  under its start (the moment of its media, or the cut when it starts within a frame of it);
  afterwards `place_lanes` moves it to where that footage plays now (its own lane if free, else the
  next free one), or clears it when the footage is gone, unless it runs on past that clip (a music
  bed stays). A lane clip the edit itself moved is left alone; Slip runs `without_linkage`. Linked
  clips show a link mark (`isLinked` in `app.js`).
- Selection: Ctrl/Shift-click adds a clip; a drag on empty timeline draws a box (`boxSelect`), a
  click there moves the playhead. Dragging one of several selected lane clips moves them all
  (`groupDrag` → `Studio::move_clips`, one undo step); several selected main clips move as one
  block (`mainGroupDrag` → `Studio::reorder_clips`). Several selected clips share a panel
  (`severalControls`: volume, fades, opacity for all of them at once).
- Shuttle (J K L): each press steps through `SHUTTLE` (1, 1.25, 1.5, 1.75, 2, 3… 8) via
  `shuttle(rate)`; forward speeds are the real player at that speed (`play(speed)`: the sound is
  time-stretched with its pitch kept by `clipcut_core::Stretcher`, the clock runs `speed` seconds a
  second, `ahead.rs` renders every `ceil(speed)`-th frame); backward runs the playhead on the wall
  clock (`S.rate` < 0) and shows frames silently. Timeline thumbnails are a decode
  of the same video, so none are fetched while playing (`pumpThumbs`): tiles borrow a cached
  picture from a wider zoom until it stops.
- Edits stop playback (the player plays a snapshot of the sequence), except mixer changes:
  `set_mixer` hands the playing `Player` a new feed (`Player::update`) so faders are heard as they
  move. Meters come from the engine's `LiveMix` in each feed (`AudioFeed::meters`); without a sound
  device the player still mixes along the wall clock so they move.
- Captions (`clipcut-core/src/captions.rs`) follow the main track's speech: every caption's cue id
  holds its media anchor (`cc:<item>:<from>:<to>`), and main-track edits go through `group_follow`,
  which re-places them afterwards (and puts the main track's transitions back on their cuts,
  `transitions.rs`). A new edit that moves main-track clips must use `group_follow`.
  The Captions panel's style controls (`buildCapStyle` / `syncCapStyle`) send
  `set_caption_style` keys (`backgroundOpacity` is joined with `backgroundColor` into the
  engine's `#rrggbbaa`; `merge` makes a dragged slider one undo step). While playing, the
  transcript box scrolls itself (`keepWordInView`, never the panel) until the person scrolls it
  (`followWords(false)`; "Follow the playback" or the next play brings it back).
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
- Exports ask the engine for `gpuRendering` and `hardwareEncoding` `auto` (`export_as`); the GPU
  renderer never takes a software adapter (`FILMCRAFT_GPU_SOFTWARE=1` lets tests use lavapipe:
  `apt-get install mesa-vulkan-drivers`). `filmcraft-gpu` asks wgpu for no backend: the app's
  `Cargo.toml` names them (dx12/vulkan, metal), or nothing renders on the GPU. Frames that need
  nothing after rendering (`Pipeline::direct`) come back from the GPU as 8-bit (`render_8bit`),
  as NV12 for an encoder that `takes_nv12` (Media Foundation; `readout.wgsl` matches
  `rgba_to_nv12`). The Export dialog says where the time went (`export_counters`). Image
  sequences decode a run of identical files once (`ImageSequenceSource::recent`), so a graphic
  holding still is decoded and uploaded once. They are bound by the one-at-a-time H.264 encoder
  (`export/src/job.rs` stage clocks: `tests/export_speed.rs`, run with `--ignored --nocapture`);
  burnt-in captions are rendered once each (`captions::burn::cached_caption`). On Windows the
  hardware H.264 encoders are NVENC, then Media Foundation's encoder MFTs (`platform/src/
  media_foundation/encode.rs` and `export.rs`: Intel Quick Sync, AMD...); `FILMCRAFT_MF_SOFTWARE_ENCODER=1`
  lets Microsoft's software MFT stand in (Windows CI's `--export-test` step). A hardware encoder
  failing partway makes the interface export again with `hardware: false`.
- Keyframes: a diamond on the timeline singles one out (`S.kf`); Delete and dragging go to
  `Studio::delete_keyframes` / `move_keyframes` (every property keyed at that time, the sound's
  volume too). A main clip's own fades are one-sided transitions on its A1 sound (`fade_alone`), and
  `transitions::lay_out` keeps them on the clip's edges through main-track edits.
- After a property's last keyframe it goes back to its first keyframe's value: one more keyframe a
  frame later that `returns.rs` keeps (listed in a clip marker named "ClipCut return", hidden from
  `keys_of`/`keyframes_at`). Every keyframe edit runs inside `with_returns` (take them off, edit,
  put them back); a new keyframe edit must too. `hold` in that marker turns them off
  (`Studio::set_hold`). Ducked music's volume gets none.
- Two-mic sides (`channels.rs`) are the engine's Channel Mixer (`channel_mixer_a`) on the sound clip;
  `channel_levels` measures the sides without it. Keep pitch (`timing.rs`) is the sound clip's
  `TrackItem::maintain_pitch`: the engine time-stretches it (WSOLA, `audio-dsp/src/stretch.rs`,
  read in `render/src/audio.rs` `stretched`, whose chain of frame shifts carries on block to block),
  never resampling, so voices stay clean. Reversed clips still get the engine's Pitch Shifter,
  chained per octave. `fit_pitch` sets either after every speed or reverse change (a new speed edit
  must call it), and `open_project` turns 0.4's Pitch Shifters into the flag (`migrate_pitch`); a
  clip marker "ClipCut pitch" turns it off. Freeze is the engine's
  Insert Frame Hold Segment after a razor of the clip and its sound, then `pack_main`. Remove a
  colour (`key.rs`) is Ultra Key; `pick_colour` renders the frame without keys.
- Stills: by default of the whole video (`still_png` / `save_still` with no item: `Studio::still_feed`,
  everything on screen at the video's size from the originals, scrubbed along the timeline); "This
  clip's footage" is `Studio::still_source`'s media frame at the file's own size from
  `Studio::stills` (originals, never proxies). Thumbnails gets either through `CS.hosted.photo`.
- Remove a colour's Pick takes the colour from the player, or from a clip's pictures on the
  timeline (`pickColourFromStrip`).
- Remove background (Thumbnails) runs in the page (Clip Studio's `js/cutout.js`: ONNX Runtime Web,
  WebGPU else WebAssembly, so the CSP has `'wasm-unsafe-eval'`); the model its Quality picks
  (Standard ISNet 178 MB; Best / Best for people BiRefNet 973 MB, graphics card only) comes from
  `HOST.model(name)` → `cutout_model` (`src/cutout.rs` `MODELS`, found by name: downloaded once into
  the models folder, checked by size and SHA-256 on every read). The pins in `cutout.rs` and
  `cutout.js` must match; Windows CI's `--model-test` step downloads Standard and checks the listed
  size of the others. Any photo on the timeline has Remove background too (`cutoutControls`): the
  original's `still_png` goes to `CS.hosted.cutout` in the Graphics page (loaded unseen), and
  `cutout_clip` saves the PNG in `Cut-outs` beside `Graphics` and swaps it into the clip in place
  (`cutouts.rs`: same size only; a clip marker "ClipCut cut-out" keeps the original's item, for
  Put background back, `ClipView.cut_from`).
- Waveforms are drawn as loud as they will play (clip and lane volume, `playGain`); past full
  scale their bars get red edges (`--sound-loud`), and the Sound panel says by how much, with
  Lower to fit (`loudHint`, `overBy`).
- Copy and paste (`clipboard.rs`): `Studio::copy_clips` keeps clips and their linked partners,
  `paste` puts copies at the playhead (lane clips keep lane and spacing or take a free lane; main
  clips go into the main track after the clip under the playhead). Copies play the same media, so
  graphics are never rendered again; a graphic's picture and its sound are linked
  (`link_together`), and Update Graphic replaces only the clip and its linked sound.
- ↑/↓ go to the previous/next event (`events()` in `app.js`: main-track cuts, the end, every
  clip's start and end, keyframes, transition edges).
- Keys: every shortcut is an entry of `ACTIONS` in `app.js` (id, default `keys`, `fn`); the keydown
  handler looks presses up by `comboOf(e)` ("Ctrl+Shift+Z": letters by layout, other keys by
  `e.code`). A new shortcut is a new entry, never a key test in the handler. The user's changes are
  `shortcuts.json` in the config folder (`load_shortcuts` / `save_shortcuts`); tooltips that name a
  key use `data-tip` + `data-keys` (or `keyText(id)`), so they follow them. A key taken off an
  action it was moved to goes back to its own (`takeKeyOff` / `sendKeysHome`); saved files carry
  `_version` 2. `ui-tests/shortcuts.js` runs this logic under Node (Windows CI): keep it passing.
- Find and replace (`replace.rs`): the search runs over the main words joined by spaces;
  `findMatches` in `app.js` must keep the same rules as `find_in`, since Replace sends the match's
  number. It rewrites the media transcripts (`transcript.set`) and the captions in one step.
- Extract Audio (`Studio::extract_audio`, `lanes.rs`) moves a main clip's sound to a sound lane and
  keeps it linked: sound linked to a main picture is the main track's wherever it sits
  (`main_links` / `is_main_sound`: `main_words`, `main_sound`, `on_main` count it), and
  `keep_sound_in_step` (in `group_follow`) puts it back in step after main-track edits, since lanes
  don't ripple. `unlink_sound` parts it (it leaves the transcript); `link_sound` joins it back to a
  picture it is in step with. Captions over words that play only on an unlinked lane clip stay on
  it (`lane_sound` in `captions.rs`).
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
