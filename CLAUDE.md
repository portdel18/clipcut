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
  Its tests build with the `ci` profile (release's optimisation without its LTO, which every one
  of ~25 test programs paid for at link time: 2 of the 2.5 hours); the installer stays release.
  Only main's runs save the build cache (`save-if`) and they are never cancelled, so each PR
  starts from main's; bump `shared-key` in both workflows when what is built changes.
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
  `export_choices`; `export(path)` stays for the self-test. "Even out the loudness" (`ExportChoice::
  loudness` / `loudness_on`, the picker's `#export-loud`, remembered per choice) sends the engine's
  `loudnessLufs` (`LOUDNESS_LUFS`, −14); `JobStatus::loudness` reads what it measured and did.
- Titles are the engine's graphics templates (`graphics.template.*`); a clip made from one has a
  template link (`ClipView.title`) and is edited through its controls, never the text keys
  (its first layer may be a shape). The user's own titles live in "Graphics Templates" beside the
  engine's preferences file, so `restore` calls `Studio::set_data_dir` with the app data folder.
- The player's frames come from `FrameFeed` (`clipcut-core/src/preview.rs`: a project snapshot
  that renders on any thread at a scale snapped to 1, ½, ¼, ⅛ by `preview_scale`). While playing,
  `apps/clipcut/src/ahead.rs` renders ahead on 2–3 workers and `frame` serves from it without the
  studio lock. It draws about 30 pictures a second at any speed and frame rate (`render_step`: a
  60 fps video shows every other frame while playing; the sound has every sample). Proxies are the
  engine's (`media.createProxies`, beside the project, or in the data folder while it is unsaved,
  never beside media on a slow drive); `Studio::poll` attaches finished ones, and the Media panel
  calls it when the job ends. Heavy videos (`MediaView.heavy`, `heavy()`: 4K, or more than 1080p
  at 30 fps) get one by themselves, one at a time (`autoProxies`, off with
  `clipcut.autoProxies`), each used when made. Phone 4K 60 decoded on the processor runs at 5–12
  fps (HEVC 10-bit the slowest); on Windows HEVC reaches the GPU only with Microsoft's HEVC Video
  Extensions (`hevc_hardware`, the Media panel says so). A proxy being made waits while the
  player plays or the check runs (`pause_job` → the engine's `Progress::paused`, held between
  frames in `Exporter::step`) and renders on the GPU when there is one (`proxies.rs`).
  **Check playback** (Media panel, `check.rs`, `playback_check`) measures on the user's computer
  which decoder each video gets (`hw_stats` frames by hardware; the engine's "declined" log lines,
  kept by `check::keep_log`), its decode fps and its drive's speed, with a report to copy: ask the
  user for it before guessing at playback problems on their hardware. The first report (Intel Arc
  140V, S26 4K 60 HEVC 10-bit HLG) showed the GPU decoding but only 21 fps: the full 4K P010
  readback and its conversion on the CPU were the bottleneck. In draft mode (playback at ½ or
  less; not exports or proxies) the Media Foundation decoder reads pictures 1400 px or more on their short side
  back at half size each way (`biplanar::to_frame_halved`, marked draft so exports and stills
  never get them; `HybridDecoder::set_draft` reaches the hardware decoder; `GopCache::finish`
  scales a clean-aperture crop for them, `region_within`), and logs its decode / copy back /
  convert time per picture every 240 pictures, which the report's decoder notes show. The
  second report: decode 57 fps (decode 1.2–2.8 ms, copy back 2.3 ms, convert 5–7 ms a picture)
  but playback still choppy. The check also times the whole player now (`check::player`: the
  timeline's frames at the player's size, draft, `render_step`, 3 workers) against the pictures a
  second it shows. The 0.4.8 report: decode 80 fps, player 21 of 30 drawn, because the CPU
  drawing converted every 4K sample to RGB while shrinking it; draft drawing now shrinks in Y'CbCr
  first (`box_decimated` in `colorman::decode` and `plain_layer`, as the GPU path does) and
  converts each drawn pixel once. HLG conversion to SDR was measured right (exact BT.2100 bars: within 2 %,
  export = player); a duller look next to the phone is HDR shown as HDR there. Next to CapCut it looked too saturated: BT.2390 on max(R,G,B) as a ratio
  keeps bright colours' full saturation, so HDR video (PQ, HLG; not log) loses what per-channel
  mapping would take from them (`HIGHLIGHT_DESAT`, also written out in `composite.wgsl`). At half
  (0.4.9) the 0.4.10 colour check still had the player 13 % more saturated than the ITU
  conversion on the user's footage (bright test footage ×1.36); at 1.0, ×1.03 (mid-tones ×1.00
  either way).
- The 0.4.10 playback report: one worker in order 12.5–20 pictures a second but the player's three
  together 3.7–8, with 17 jumps back to a keyframe and no cache hits in two seconds. Timeline
  thumbnails (and the Media panel's) read the same source as the player, so each one moved the
  player's decoder and pushed its pictures out of the cache. `Studio::thumbs` now reads every
  video without a proxy through a source of its own (`ThumbProvider`, kept in
  `Studio::thumb_sources`); `tests/thumbs.rs` (4K: 47 pictures decoded for 3 when shared, 9 now).
  Check playback also stops the thumbnails while it measures (`PX.checking` in `pumpThumbs`).
  The 0.4.11 report: 20–22 of 30 drawn on 4K 60, every frame decoded, read back and converted
  (~7 ms each) though only every other one is shown. `GopCache` now tells the decoder playback's
  rhythm (`keep_for`: the step between recent draft requests, in frames; `VideoDecoder::set_keep`,
  `Keep::wants`) and Media Foundation decodes the others without reading them back (`left_out`,
  "not read back" in the decoder notes). An exact request for a sample fed in draft mode starts
  again (`draft_to`), so a paused frame is the one asked for.
- The 0.4.9 report: decode 63 fps but the player drew 2.7 of 30 (0.4.8: 21), not reproduced here
  (no Media Foundation; drawing on the CPU is as before, ~100 ms a 1080×1920 HLG picture on 4
  cores). Since then the check's player line also gives one worker in order, the drawing alone,
  and the decoders' counts meanwhile (`played_words`: by the graphics card, seeks, cache hits,
  new hardware decoders, fallbacks): read those before changing the player. By hand:
  `tests/player_speed.rs` and `check::tests::check_a_clip` (`PLAYER_CLIP` / `CHECK_CLIP`).
- **Check colour** (Media panel, `colour_check.rs`, `#colour-check`): for each video at three
  moments of its first clip, HSV saturation (mean, 90th percentile) and Y'CbCr chroma of the
  footage's own signal, of the ITU conversion to SDR BT.709 (no tone mapping: the colorimetric
  target) and of the player's picture, with the ratios; a report to copy, like Check playback.
- Keyboard focus: buttons never take focus (`mousedown` is prevented), and a click on one takes it
  away from a box; sliders, tick boxes, choices and numbers give it back on `change`. Shortcuts are
  held only while typing (`typingIn`: text and number boxes, notes, dropdowns; a slider keeps its
  arrow keys).
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
- Split at scene changes (`scenes.rs`, `#scenes-prop` beside Freeze, app `scenes.rs`): the engine's
  scene detector (`filmcraft_render::scene`) over the clip's frames, read off the studio lock from a
  source of its own at 256 px, draft (`SceneScan::run`, `scene-progress` events, `cancel_scenes`);
  then `split_at_scenes` razors the clip at each change through `group_follow` (one undo step), not
  the engine's `clip.sceneEditDetection`, whose plain razor skips captions and linkage.
- Speed curves (CapCut's curve speed; `ramps.rs`, `rampControls` in the Speed section): the engine's
  Time Remapping Speed keyframed at media times on the picture and its sound (a sound clip gets a
  `time_remap` instance of its own). The engine follows it (`crates/project/src/ramp.rs`): with
  one, `source_time_at` / `source_out` integrate 1 / speed over a grid fixed in media time (so
  splits carry on exactly), razor and head trims hand the media over through `media_after_start`,
  and the sound is read sample by sample at the moving speed (`audio.rs` `ramped`, pitch follows).
  `set_ramp` keeps the clip's footage: `clip.speedDuration` gives it the ramp's length (main track
  rippled), then the keyframes go on at speed 1. Any one-speed or Reverse edit calls `drop_ramps`
  first. Code that maps media time to timeline uses `TrackItem::time_to_media`, never `/ speed`.
- Markers with notes (`markers.rs`, the Markers tab `#panel-markers`, `#marker-btn`, `M`, app
  `markers.rs`): the engine's sequence markers (`custom_color`, `comment` = notes), edited straight
  through `edit_sequence`, one undo step each (notes typing merges). `group_follow` holds them
  (`hold_markers`: the main clip and media moment under each) and puts them back over that footage
  (`place_markers`, the linkage rules); a marker whose footage is gone stays put. Marker commands
  don't stop playback, and the interface applies them while playing without redrawing the player
  (`applyMarkers`). `chapters` makes YouTube chapters from named markers with YouTube's rules.
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
- Background fill (CapCut's Canvas › Blur; `fill.rs`, the Background section, `fillControls`): the
  engine's Blur Fill effect on a picture clip (`render/src/blur_fill.rs`, hooked in `item_layer_ex`
  after the clip's effects): its own picture zoomed to cover the frame, blurred, darkened, under it,
  and its corners rounded. Not a GPU effect, so `plan.rs` renders such a clip on the CPU (exports
  match). It sits in its own `Canvas` folder, outside Premiere's effect list (`vfx.rs` catalogue test).
- Voice cleanup (`voice.rs`, `voiceControls` in the Sound section): Reduce noise and Enhance voice are
  the engine's DeNoise and Enhance Speech put straight on the sound clip (not Essential Sound, whose
  audio types would clash with ducked music), after its other effects, noise first; their delay is
  compensated by the engine. DeNoise ignores digital silence and lets the first 0.25 s of sound
  through while it learns the floor (`audio-dsp/src/effects/restoration.rs`), so it works from a
  clip's start and right after a seek.
  Reduce noise's AI (the default, `Voice::ai`) is DeepFilterNet 3 (`ai_denoise.rs`, the official
  `deep_filter` crate from its git tag, MIT/Apache, model built in): an engine audio effect ClipCut
  adds with `filmcraft_audio_dsp::register_effect` (its project entry is in the `Application`
  folder, skipped by the engine's DSP catalogue test), so it is latency-compensated, cached and
  exported like any other. The model isn't `Send`: each effect leases a worker thread from a small
  pool (`lease`), which starts from a clone of the loaded model. 48 kHz, 1 or 2 channels only;
  anything else passes through. Its latency is a 480-sample frame plus the model's 1440. It runs with DeepFilterNet's command-line settings
  (`runtime`: thresholds −15 / 35 / 35 dB, post-filter 0.02), not the library's defaults, which
  skipped deep filtering on most speech and made it duller than the noisy input (measured on its
  repo's real speech and noise).
  The engine reuses a clip's effect chain only for a request that starts where the last one ended
  (`audio_fx::process`); any other request builds the chain again with its warm-up (0.5 s of
  sound through the AI model, 1 s for DeNoise). The player reads a little past each chunk to
  interpolate, so `playback.rs` keeps that overlap itself (`Held`) and asks the mix only for new
  samples: overlapping reads once made AI noise reduction mix at 0.2× real time and slowed the
  whole player.
- Voiceover (`narration.rs`, the Audio tab's `#vo-*`, app `narration.rs`): the engine's text to speech
  (`filmcraft_tts`, with its `kokoro` feature) called from clipcut-core, not the engine's `tts.create`
  (which places clips without ClipCut's lanes and keeps voices in the engine's own data folder).
  Natural voices download into ClipCut's models folder (`download_voices`, `voice-progress`,
  `cancel_voices`); `speak` runs off the studio; the WAV goes to `Voiceovers` beside the project
  (`voiceover_dir`) and onto a sound lane through `add_to_lane`.
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
- Colour boxes: WebView2's colour pop-up dropper does nothing, so `index.html` loads Clip Studio's
  `graphics/js/dropper.js` (with `data-colour-hex` on `<html>`): every `input[type=color]`, also
  ones made later, gets a hex box (`.colour-hex`) and a dropper that reads the picture clicked
  (the player, timeline pictures, media cards; `#overlay` is `data-nopick`) and fires `input` +
  `change` on the box, so its own handlers send it. Change it in Clip Studio and sync.
- The library's tab rail (`.rail`) scrolls when the timeline leaves it too short.
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
