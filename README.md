# ClipCut

A desktop video editor for Windows with a friendly, CapCut-style interface and a professional engine
underneath. ClipCut is built on [FilmCraft](https://github.com/storytold/filmcraft), an open-source,
pure-Rust editing engine (its own H.264, HEVC, ProRes, DNxHR, VP9, AV1, AAC and Opus codecs, exact
frame timing, colour, audio mixing and export), and will bring in
[Clip Studio](https://github.com/portdel18/clip-stu)'s news graphics.

> **Status: Phase 2, Clip Studio inside.** A magnetic main track you split, trim, reorder and
> ripple-delete; transitions on its cuts; lanes for overlays, text, music and voiceover; a
> Properties panel; vertical and square canvases; auto-captions from the speech (on this computer)
> that follow every cut, with transcript cutting and subtitle files; and a **Graphics** tab with
> Clip Studio's lower thirds, maps, numbers, timelines and more, whose graphics become clips you can
> open again and change. See [the roadmap](#roadmap).

## Install on Windows

1. Download `ClipCut_<version>_x64-setup.exe` from the latest release on the repository's
   **Releases** page. (Builds of work in progress are under the **Actions** tab: open a run of the
   **ClipCut Windows** workflow, or a pull request's checks, and download
   **ClipCut-windows-installer**.)
2. Run it. It installs for your user account only, adds ClipCut to the Start menu, and fetches
   Microsoft's WebView2 if Windows doesn't already have it (Windows 11 does).
3. If the installer isn't code-signed, Windows shows **"Windows protected your PC"** the first time:
   choose **More info → Run anyway**.

**Updates.** ClipCut checks for a newer release a few seconds after it starts and shows **Update to
x.y.z** in the top bar: it saves the project, downloads and installs the new version, and opens it
again. A release made with an update key is also checked against that key; without one, ClipCut
installs the release's installer once it matches the checksum GitHub lists for it. **About ClipCut** (the name at the top left) shows the version and checks by hand. How
releases, update keys and code signing are set up: [RELEASING.md](packaging/clipcut/RELEASING.md).

Your footage stays on your computer: ClipCut reads it from disk and writes exports where you choose.

## Using it

**Adding clips.** **Import** (`Ctrl+I`) or drop videos on the window: they go to the end of the main
track, in order. Drag a Media card onto the timeline to put it between clips, or press its **+** to add
it at the end again.

**The main track is magnetic**, like CapCut's: its clips always sit back to back, so deleting or
trimming a clip closes the gap and everything after it moves up. Each clip's own sound moves with it.

**Lanes.** Drag a Media card *above* the main track to lay it over the clips there (an overlay, for
b-roll or picture-in-picture). The **Text** tab adds text at the playhead in a style to start from
(Plain, Outline, Box, Shadow, Caption, Title, Strap, Soft). The **Audio** tab imports music or a
voiceover onto a lane below the main track. Lane clips stay where you put them when the main track
changes; drag them along or onto another lane, trim their edges, and delete them without closing up.
The switch at the left of each lane hides it or mutes it (the main track's switch mutes its sound).

**Properties** (right panel) follow the selection: position, scale, rotation and opacity (or drag
the picture or text in the player, with the wheel to scale it and the centre lines pulling it in);
Fit, Fill and Centre; volume; fades on any clip's sound (the main track's too); **Extract audio**, which puts a clip's sound on a sound lane as a clip of its own (captions stay on its words); speed (0.5× to 3×, or any percent); and
for text the words, font, size, bold and italic, alignment, colour, outline, box and shadow. A slider
drag is one undo step. **Ratio** (under the player) turns the canvas into 9:16 for TikTok, Reels and
Shorts, 1:1 or 4:5; clips are refitted to the new shape.

**Two-mic recordings.** A clip recorded with a microphone on each side (one person on the left, one
on the right) chooses how its sides play under Sound: **As recorded**, **Left mic** or **Right mic**
(that side in both ears), or **Both mics** (both sides mixed into both ears), with a level for each
mic. ClipCut measures the two sides and offers the fix: "Each side has its own voice" for two people,
"Only the left side has sound" for one microphone, and **Even out** sets the louder voice down to the
quieter one.

**Speed, reverse, freeze.** Speed keeps the voice's pitch (switch it off under Speed for the tape
effect); **Reverse** plays the clip backwards; **Freeze** (the toolbar, or `F`) puts a 3-second still of
the frame at the playhead into the video and pushes the rest along (trim the still to change its
length). The sound pauses during the freeze, so speech stays with its picture.

**Remove a colour.** Under Properties, a video or picture clip can lose a green screen (or any flat
colour): tick **Remove a colour**, or press **Pick from the video** and click the colour in the
player. **Gentle**, **Normal** or **Strong** sets how much is removed, **Soft edges** feathers the
cut, and **Remove tint** takes the screen's colour off hair and edges. Put the clip on a lane over
the background it should stand on.

| Key | What it does |
|---|---|
| `S` | Split at the playhead (only the selected clips, if any are selected) |
| `F` | Freeze the frame at the playhead for 3 seconds |
| `Q` / `W` | Split and delete everything to the left / right of the playhead in that clip |
| `Delete` | Delete the selected clips; the main track closes up, lanes keep their gaps |
| Drag a clip | Main track: move it to another place in the order. Lanes: move it in time or to another lane |
| Drag a selected clip's edge | Trim it; the player shows the new first or last frame |
| `,` / `.` | A frame earlier / later (`Shift`: 10 frames): the selected cut moves (one clip grows as the other shrinks), or the selected lane clips move |
| `Alt+,` / `Alt+.` | Slip the selected clip: the media a frame earlier / later, in the same place and length |
| `N` | Snapping on or off (hold `Shift` while dragging to skip it once) |
| `Ctrl+Z`, `Ctrl+Shift+Z` / `Ctrl+Y` | Undo, redo: every edit (and every slider drag) is one step |
| Click, `Ctrl`/`Shift`+click, `Ctrl+A`, `Esc` | Select a clip, add to the selection, select all, select none |
| `Space` or `K` | Play, pause |
| `←` `→` | A frame back or on (`Shift`: 5 seconds); `J` / `L`: a second |
| `↑` `↓` | The previous or next cut; `Home` / `End`: the start or the end |
| Pinch on the touchpad, `Ctrl`+wheel, `Ctrl+=` / `Ctrl+-`, `Ctrl+0` | Zoom the timeline at the pointer, in and out, fit it all |
| `Ctrl+F` | Find and replace in the transcript |
| `Ctrl+/` | Keyboard shortcuts |

These are the default keys. **Keyboard shortcuts** (the keyboard button in the top bar, or `Ctrl+/`)
lists every action with its keys: press + beside one and then the keys you want, × to take a key off,
or **Default** to go back. A key another action had moves to the new one. Your keys are kept in
ClipCut's settings folder (`shortcuts.json`), and the tooltips show them.

**Graphics.** The **Graphics** tab opens [Clip Studio](https://github.com/portdel18/clip-stu)
over the editor: Underline, Map Zoom, Split Screen, Scroll Zoom, Timeline, Numbers, Audiogram and
Lower Thirds. Make a graphic as you would there, then press **Add to video**: it renders with Clip
Studio's own drawing, in the video's shape, and lands on a lane at the playhead, fitted to the frame
(a lower third keeps its transparent background, over the video). Its frames, sound and settings go
in a `Graphics` folder beside the project. Select a graphic and press **Edit in Graphics** to open it
again with its settings; **Update in video** puts the new version in its place, at the same time
and size. Photos, videos and sound a graphic used aren't kept: add them again when you change it. A lower third is designed over your
video: Graphics shows the frame at the playhead behind it (without the graphic being changed),
never in what it renders. When **Ratio** changes the video's shape, ClipCut offers to make the
graphics in the old shape again in the new one, each in its place. Drop a photo, video, PDF or
sound file on a Graphics tool's preview or file box and it goes there (not onto the timeline); a
thumbnail, a render or presets saved from Graphics ask where to save them.
Maps need the internet for their tiles and searches; scanned-PDF text recognition (Scroll Zoom)
isn't available inside ClipCut.

**Keyframes.** Every picture and text clip's scale, position, rotation and opacity, and every
clip's volume, can change over time. Press the ◇ beside a property: it keeps its value at the
playhead as a keyframe (◆), and changing the property at another time makes the next one, so a
clip fades, moves or grows between them. The Keyframes row goes to the previous or next keyframe
and makes the motion **Smooth** (easing in and out) or steady; the clip shows its keyframes as
diamonds on the timeline: click one to select it (the playhead goes there and the panel says what is
keyed), **Delete** removes just that keyframe, and dragging it moves it along the clip. Keyframes
stay on their frames of the clip when it is moved or trimmed. After its last keyframe a property
goes straight back to its value at the first, so a zoom in for a few seconds is over by itself;
**Hold the last value** keeps the last keyframe's values to the end of the clip instead.

**Colour.** A picture clip's Properties have an **Adjust** section: exposure, contrast,
highlights, shadows, whites, blacks, temperature, tint, saturation, vibrance, fade, sharpen and
vignette (double-click a name to reset it), with **Reset** and **Apply to all clips** (the main
track). The **Filters** tab shows the clip's own picture under each look (Teal & Orange, Warm Film,
Cool Blue, Bleach Bypass, Faded Matte, Monochrome, Golden Hour, Night) with a strength slider. All
of it is the engine's Lumetri Color, so the export matches the player.

**Scopes.** **Scopes** at the top of the Properties panel shows the player's frame as a waveform,
an RGB parade, a vectorscope (with the colour-bar targets) or a histogram, updated as you scrub
and play.

**Text animations.** A text clip's Properties have an **Animation** section: how it comes in
(fade, rise, drop, slide, pop, zoom, typewriter, word by word) and goes out (fade, rise, sink,
slide, shrink, zoom, erase, word by word), and how long each takes. Picking one plays it once.
They stay on the clip's ends when it is trimmed, and follow new words when the text changes.

**Titles.** The Text tab's **Titles** are ready-made designs (lower thirds, titles, end credits,
callouts, a news ticker): click one to add it at the playhead, then change its words, colours, font
and what shows in Properties. **Save as title** keeps any text, with its look and animations, under
**My Titles** to use again (the × on one removes it).

**Ducking.** Music on a lane can **Lower under speech**: it goes down (15 dB, or as far as you
set) wherever the main track or another lane has sound under it, with short fades. The dips are
volume keyframes on the music; **Update** finds the speech again after you move or trim clips.

**Beats.** A music clip's Properties can **Mark beats**: the engine's beat tracker finds them
and they show as dots along the clip, and clip edges and the playhead snap to them. **Beat at
playhead** adds one by hand (or takes away the one there); **Clear** removes them. They move with
the clip.

**Export choices.** **Export** asks where the video goes: as the project is, TikTok / Reels / Shorts
(1080 × 1920), YouTube (1080p or 4K, in the video's shape), a small file to send, a ProRes master,
or the sound alone, each with its size and roughly how big the file will be. A 16:9 video for
TikTok gets a note to change Ratio to 9:16 first.

**Mixer.** The **Mixer** tab has a strip for the main track's sound, one for each sound lane and
one for the Mix (everything, as exported): volume and pan, Mute and Solo, and a level meter that
moves while the video plays (red past 0 dB, where the sound distorts). Faders change the sound as
it plays; a fader drag is one undo step.

**Precise trimming.** A clip's Properties have a **Trim** section: start or end to the playhead,
**Slip** (other frames of the media in the same place) and, on lanes, **Move** a frame at a time. A
selected cut has **Move cut**, a roll: one clip grows as the other shrinks, so the video keeps its
length, linked sound, captions and transitions following.

**Smooth playback.** The player renders at the canvas size or a whole fraction of it (½, ¼),
whichever fills it, and renders ahead while the video plays. The quality menu under the player
(Auto, Full, ½, ¼) trades sharpness for smoothness on a slow computer. For big or 4K footage,
**Make proxies** in the Media panel makes small ProRes copies in the background (in the project's
folder) and plays from them; **Use for playback** turns them on or off. Exports always use the
originals.

**Platform overlays.** The player's **Overlay** menu shows what TikTok, Instagram Reels or YouTube
Shorts cover on a 9:16 video (the same drawing as Clip Studio's), or the safe zones on any shape.
It is only in the player, never in an export.

**Transitions.** Hover a cut on the main track and click its square, then pick one of sixteen in
the **Transitions** tab (Mix, through black or white, Flash, Blur, Zoom, Push, Slide, Whip, Wipe,
Circle, Glitch, Light leak, 3D spin; each tile plays a preview when you point at it). It plays over
the cut, half in each clip, so the video keeps its length; where a clip has no footage past its
edge, its first or last frame holds. Set its length in Properties, **Apply to every cut**, or press
`Delete` for a plain cut again. When both clips have sound to spare past the cut, the sound
crossfades too. Transitions stay on their cuts as you trim, split, reorder and change speed, and
go when their two clips no longer meet.

**Captions.** The **Captions** tab's **Auto captions** listens to the main track's speech and lays
it out as short captions on a lane at the top. The speech model (Whisper, from OpenAI, run by the
engine in pure Rust) downloads once, 150 MB to 970 MB by the accuracy you pick, into ClipCut's
data folder, and runs on this computer: nothing is uploaded. Captions follow the speech: cut, trim,
reorder or speed up the clips and each caption moves with its words, loses the words that were cut,
and goes when all of them have. Pick a look (Clean, Box, Yellow, Big, Top, Serif), edit a caption's
words or times in Properties (or drag it on its lane), and **Save subtitles** writes an `.srt` or
`.vtt`. Shown captions are burnt into exports.

**Cutting by the words.** The Captions tab's transcript is the main track's speech: click a word to
go to it, drag across words (or Shift+click) and press `Delete` to cut them out of the video.
**Remove filler words** cuts um, uh and the like; **Shorten pauses** trims every pause over half a
second. The main track closes up each time, and the captions follow.

**Find and replace** (`Ctrl+F`, or the button above the transcript) searches the transcript as text:
part of a word, or a phrase across words, with **Match case** if you want it. Enter goes to the next
match (Shift+Enter the one before), and the playhead follows. **Replace** changes the highlighted
match and **Replace all** every one, in the transcript and in the captions over it (a caption you
typed over gets the change in its own words), as one undo step. Nothing is cut from the video.

**Projects save themselves.** The first edit gives the project a folder, `Documents\ClipCut\<first
clip's name>`, and every change after that is saved a moment later. ClipCut reopens the last project
when it starts. **New** (`Ctrl+N`) starts another, **Open** (`Ctrl+O`) picks a saved one.

**Export** (`Ctrl+E`) asks where the video goes (see Export above), then saves an H.264 MP4 (a
ProRes `.mov` for the master, a `.wav` for the sound) with progress and Cancel. The graphics card
composes and encodes the video where it can (NVIDIA's NVENC, Intel Quick Sync and AMD's encoders
through Windows' Media Foundation); if its encoder fails partway, ClipCut exports again with its own.

## Roadmap

| Phase | What it adds |
|---|---|
| **0. Prove it** (done) | Open, play with sound, scrub, export; the Windows installer from CI |
| **1. Editor v1** (done: 1a cutting, 1b lanes and properties, 1c captions, 1d transitions) | Split, trim, ripple delete, a magnetic main track plus overlay, text and audio lanes, speed, transitions, vertical and horizontal canvases, undo, autosave. First workflows: cutting down interviews and captioned explainers (auto-captions with styles, transcript cutting) |
| **2. Clip Studio inside** (this) | A Graphics tab with Clip Studio's tools (lower thirds, map zoom, numbers, timelines, underline); graphics become clips you can reopen and edit; TikTok, Reels and Shorts overlays in the preview |
| **3. Pro panels** (done: 3a keyframes, 3b colour and scopes, 3c the mixer and precise trimming) | Keyframes, colour, the audio mixer, scopes and precise trimming, from the engine |
| **4. CapCut extras** (done: 4a text animations and ducking, 4b beats and quick exports, 4c titles) | Caption styles, text animations, templates, beat markers, music ducking, quick social exports |
| **5. Polish** (5a smooth playback and proxies; 5b graphics that follow the video; 5c releases and updates: this) | Live graphics, proxies, speed work, signed installers and updates |

## How it is built

| Layer | Where | What |
|---|---|---|
| Engine | `crates/*` (from FilmCraft) | Decoding, the timeline model, rendering, effects, audio mixing, export |
| Façade | `crates/clipcut-core` | The engine operations the app calls: the magnetic main track and the lanes around it (`timeline.rs`, `lanes.rs`), clip properties, canvas and waveforms (`props.rs`), captions and transcript cutting (`captions.rs`), find and replace in the transcript (`replace.rs`), transitions (`transitions.rs`), graphics from the Graphics tab (`graphics.rs`), keyframes (`keyframes.rs`), colour and scopes (`colour.rs`, `scopes.rs`), the mixer and precise trimming (`mixer.rs`, `trimming.rs`), text animations (`animations.rs`), ducking (`ducking.rs`), beats (`beats.rs`), quick exports (`exporting.rs`) and titles (`titles.rs`), the player's frames and proxies (`preview.rs`), each edit one undo step; projects, preview frames and thumbnails, playback audio, export jobs |
| App | `apps/clipcut` | A [Tauri](https://tauri.app) window: Rust commands in `src/`, the interface in `ui/` (HTML, CSS and JavaScript, no build step) |

The sound card is the playback clock; the interface shows the frame for that time. While the
video plays, worker threads render the next frames from a snapshot of the sequence (so they never
wait for the project), and a frame that is late is skipped rather than slowing the picture down.

## Building

Rust (stable, 1.95 or later) is all the engine needs. The app also needs the
[Tauri prerequisites](https://tauri.app/start/prerequisites/): on Windows, the Microsoft C++ Build
Tools and WebView2; on Ubuntu, `libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev
librsvg2-dev libasound2-dev libxdo-dev libssl-dev`.

```sh
cargo test -p clipcut-core                         # the façade, against a clip ffmpeg generates
cargo run -p clipcut -- a.mp4 b.mp4                # the app, starting a project with these clips
cargo run -p clipcut -- --self-test in.mp4 out.mp4 report.json   # open, preview, sound, export; no window
cargo run -p clipcut -- --caption-test speech.mp4 models/ whisper-tiny report.json out.srt   # real-speech captions
CLIPCUT_TEST_TRANSCRIPT=words.json cargo run -p clipcut -- clip.mp4   # the Captions tab without a model
cargo run -p clipcut -- --verify-update setup.exe setup.exe.sig 0.2.0   # a release's update signature (needs CLIPCUT_UPDATER_PUBKEY at build time)
cd apps/clipcut && npx @tauri-apps/cli@2 build     # the installer (target/release/bundle/)
```

CI (`.github/workflows/clipcut-windows.yml`) runs the tests, builds the installer and runs the
self-test on Windows for every pull request. Releases come from `.github/workflows/clipcut-release.yml`
(see [RELEASING.md](packaging/clipcut/RELEASING.md)).

## Credits and licence

ClipCut is MIT OR Apache-2.0, like FilmCraft, whose engine it uses: see [NOTICE](NOTICE),
[LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE). FilmCraft is made by the ArtCraft
Team and the FilmCraft contributors; its own README is kept at
[docs/filmcraft-README.md](docs/filmcraft-README.md). ClipCut is an independent project, not made or
endorsed by them. Every asset is listed in [ATTRIBUTION.md](ATTRIBUTION.md).
