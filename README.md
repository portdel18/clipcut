# ClipCut

A desktop video editor for Windows with a friendly, CapCut-style interface and a professional engine
underneath. ClipCut is built on [FilmCraft](https://github.com/storytold/filmcraft), an open-source,
pure-Rust editing engine (its own H.264, HEVC, ProRes, DNxHR, VP9, AV1, AAC and Opus codecs, exact
frame timing, colour, audio mixing and export), and will bring in
[Clip Studio](https://github.com/portdel18/clip-stu)'s news graphics.

> **Status: Phase 1a, cutting.** Clips go on one magnetic main track that you split, trim, reorder
> and ripple-delete, with undo and projects that save themselves. Text, sound tracks and captions come
> next (1b, 1c). See [the roadmap](#roadmap).

## Install on Windows

1. Open the latest run of the **ClipCut Windows** workflow under the repository's **Actions** tab (or
   a pull request's checks) and download **ClipCut-windows-installer**.
2. Unzip it and run `ClipCut_<version>_x64-setup.exe`. It installs for your user account only, adds
   ClipCut to the Start menu, and fetches Microsoft's WebView2 if Windows doesn't already have it
   (Windows 11 does).
3. Until the app is code-signed, Windows shows **"Windows protected your PC"** the first time: choose
   **More info → Run anyway**.

Your footage stays on your computer: ClipCut reads it from disk and writes exports where you choose.

## Using it

**Adding clips.** **Import** (`Ctrl+I`) or drop videos on the window: they go to the end of the main
track, in order. Drag a Media card onto the timeline to put it between clips, or press its **+** to add
it at the end again.

**The main track is magnetic**, like CapCut's: its clips always sit back to back, so deleting or
trimming a clip closes the gap and everything after it moves up. Each clip's own sound moves with it.

| Key | What it does |
|---|---|
| `S` | Split at the playhead (only the selected clips, if any are selected) |
| `Q` / `W` | Split and delete everything to the left / right of the playhead in that clip |
| `Delete` | Delete the selected clips; the gap closes |
| Drag a clip | Move it to another place on the main track |
| Drag a selected clip's edge | Trim it; the player shows the new first or last frame |
| `N` | Snapping on or off (hold `Shift` while dragging to skip it once) |
| `Ctrl+Z`, `Ctrl+Shift+Z` / `Ctrl+Y` | Undo, redo: every edit is one step |
| Click, `Ctrl`/`Shift`+click, `Ctrl+A`, `Esc` | Select a clip, add to the selection, select all, select none |
| `Space` or `K` | Play, pause |
| `←` `→` | A frame back or on (`Shift`: 5 seconds); `J` / `L`: a second |
| `↑` `↓` | The previous or next cut; `Home` / `End`: the start or the end |
| `Ctrl`+wheel, `Ctrl+=` / `Ctrl+-`, `Ctrl+0` | Zoom the timeline at the pointer, in and out, fit it all |

**Projects save themselves.** The first edit gives the project a folder, `Documents\ClipCut\<first
clip's name>`, and every change after that is saved a moment later. ClipCut reopens the last project
when it starts. **New** (`Ctrl+N`) starts another, **Open** (`Ctrl+O`) picks a saved one.

**Export** (`Ctrl+E`) saves an H.264 MP4 (or ProRes `.mov`, or a `.wav` of the sound) with progress and
Cancel.

## Roadmap

| Phase | What it adds |
|---|---|
| **0. Prove it** (done) | Open, play with sound, scrub, export; the Windows installer from CI |
| **1. Editor v1** (1a, cutting: this) | Split, trim, ripple delete, a magnetic main track plus overlay, text and audio lanes, speed, transitions, vertical and horizontal canvases, undo, autosave. First workflows: cutting down interviews and captioned explainers (auto-captions with styles, transcript cutting) |
| **2. Clip Studio inside** | A Graphics tab with Clip Studio's tools (lower thirds, map zoom, numbers, timelines, underline); graphics become clips you can reopen and edit; TikTok, Reels and Shorts overlays in the preview |
| **3. Pro panels** | Keyframes, colour, the audio mixer, scopes and precise trimming, from the engine |
| **4. CapCut extras** | Caption styles, text animations, templates, beat markers, music ducking, quick social exports |
| **5. Polish** | Live graphics, proxies, speed work, signed installers and updates |

## How it is built

| Layer | Where | What |
|---|---|---|
| Engine | `crates/*` (from FilmCraft) | Decoding, the timeline model, rendering, effects, audio mixing, export |
| Façade | `crates/clipcut-core` | The engine operations the app calls: the magnetic main track's edits (each one undo step), projects, preview frames and thumbnails, playback audio, export jobs |
| App | `apps/clipcut` | A [Tauri](https://tauri.app) window: Rust commands in `src/`, the interface in `ui/` (HTML, CSS and JavaScript, no build step) |

The sound card is the playback clock; the interface shows the frame the engine renders at that time.

## Building

Rust (stable, 1.95 or later) is all the engine needs. The app also needs the
[Tauri prerequisites](https://tauri.app/start/prerequisites/): on Windows, the Microsoft C++ Build
Tools and WebView2; on Ubuntu, `libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev
librsvg2-dev libasound2-dev libxdo-dev libssl-dev`.

```sh
cargo test -p clipcut-core                         # the façade, against a clip ffmpeg generates
cargo run -p clipcut -- a.mp4 b.mp4                # the app, starting a project with these clips
cargo run -p clipcut -- --self-test in.mp4 out.mp4 report.json   # open, preview, sound, export; no window
cd apps/clipcut && npx @tauri-apps/cli@2 build     # the installer (target/release/bundle/)
```

CI (`.github/workflows/clipcut-windows.yml`) runs the tests, builds the installer and runs the
self-test on Windows for every pull request.

## Credits and licence

ClipCut is MIT OR Apache-2.0, like FilmCraft, whose engine it uses: see [NOTICE](NOTICE),
[LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE). FilmCraft is made by the ArtCraft
Team and the FilmCraft contributors; its own README is kept at
[docs/filmcraft-README.md](docs/filmcraft-README.md). ClipCut is an independent project, not made or
endorsed by them. Every asset is listed in [ATTRIBUTION.md](ATTRIBUTION.md).
