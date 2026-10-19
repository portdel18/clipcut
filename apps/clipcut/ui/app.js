/* ClipCut interface: talks to the Rust side through Tauri commands (apps/clipcut/src/main.rs and
   editing.rs). Every edit comes back as the whole timeline (`TimelineView`), which `apply` redraws.

   The timeline is CapCut's: one magnetic main track (its clips always sit back to back, so a
   delete or a trim closes up), lanes above it for overlay video, pictures and text, and lanes below
   it for music and voiceover. Lane clips stay where they are put.

   The preview shows the frame the engine renders at the playhead; while playing, the sound card is
   the clock and the picture follows it. */
'use strict';
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const $ = s => document.querySelector(s);

const PAD = 16;          // the timeline's left padding, px
const SNAP_PX = 10;      // snapping reach
const EDGE_PX = 100;     // auto-scroll zone at the timeline's edges while dragging
const EDGE_STEP = 15;    // px per frame it scrolls by
const MAX_PX = 600;      // px per second at full zoom (about 20 px a frame at 30 fps)
const THUMB_W = 192;     // filmstrip pictures are rendered this wide
const SAVE_MS = 800;     // autosave this long after the last edit
const PEAKS_PER_SECOND = 100;
const ROW = { main: 64, video: 40, audio: 40, caption: 30, gap: 6, top: 14 };

const S = {
  tl: null,              // TimelineView from the engine
  t: 0,                  // playhead, seconds
  playing: false,
  px: 60,                // timeline zoom, px per second
  sel: new Set(),        // selected clip ids (main track and lanes; never the main track's sound)
  kf: null,              // a keyframe singled out on the timeline: {clip, t}
  cut: null,             // or the selected cut on the main track: the clip before it
  snap: true,
  frameBusy: false,      // a preview frame request is in flight
  want: null,            // the latest preview asked for while busy
  geom: new Map(),       // clip id → where a drag shows it (overrides its real place)
  dragging: false,
  els: new Map(),        // clip id → element
  rows: [],              // lanes on screen, top to bottom: {id, kind: video|main|audio, top, h}
};

/* ---------- small helpers ---------- */
let toastTimer = 0;
function toast(msg, err = false) {
  const el = $('#toast');
  el.textContent = cap(String(msg));
  el.classList.toggle('err', err);
  el.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { el.hidden = true; }, err ? 6000 : 3500);
}
// a message that starts with a file's name keeps the name as it is
const cap = s => !s || /^[^\n:]+\.[a-z0-9]{2,5}:/i.test(s) ? s : s[0].toUpperCase() + s.slice(1);
const pad = n => String(n).padStart(2, '0');
const sleep = ms => new Promise(r => setTimeout(r, ms));
const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v));
function fmt(sec) {
  sec = Math.max(0, sec || 0);
  const h = Math.floor(sec / 3600), m = Math.floor(sec / 60) % 60, s = Math.floor(sec) % 60;
  return `${pad(h)}:${pad(m)}:${pad(s)}`;
}
/** hh:mm:ss:ff */
function fmtFrames(sec, fps = S.tl ? S.tl.fps : 30) {
  const total = Math.round(Math.max(0, sec) * fps);
  const f = total % Math.round(fps), whole = Math.floor(total / Math.round(fps));
  return `${fmt(whole)}:${pad(f)}`;
}
/** mm:ss:ff, with hours only when there are some. */
const tcShort = sec => sec >= 3600 ? fmtFrames(sec) : fmtFrames(sec).slice(3);
/** A short length: 4.5s, 1:05, 1:02:03. */
function fmtLen(sec) {
  if (sec < 60) return `${(Math.round(sec * 10) / 10).toFixed(1)}s`;
  const t = fmt(sec);
  return sec < 3600 ? t.slice(3) : t;
}
/** Decodes a frame or thumb reply: width, height (u32 LE), then RGBA. */
function toImage(buf) {
  const dv = new DataView(buf);
  const w = dv.getUint32(0, true), h = dv.getUint32(4, true);
  return new ImageData(new Uint8ClampedArray(buf, 8, w * h * 4), w, h);
}
function toCanvas(img) {
  const c = document.createElement('canvas');
  c.width = img.width; c.height = img.height;
  c.getContext('2d').putImageData(img, 0, 0);
  return c;
}
const svg = d => `<svg viewBox="0 0 24 24" aria-hidden="true">${d}</svg>`;
const ICON = {
  eye: svg('<path d="M2 12s3.6-7 10-7 10 7 10 7-3.6 7-10 7S2 12 2 12z"/><circle cx="12" cy="12" r="3"/>'),
  eyeOff: svg('<path d="M3 3l18 18M10.6 5.1A10 10 0 0 1 12 5c6.4 0 10 7 10 7a17 17 0 0 1-3.2 4M6.1 6.1C3.6 7.8 2 12 2 12s3.6 7 10 7a9.7 9.7 0 0 0 4.9-1.3"/>'),
  sound: svg('<path d="M4 9h4l5-4v14l-5-4H4z"/><path d="M16.5 8.5a5 5 0 0 1 0 7M19 6a8.5 8.5 0 0 1 0 12"/>'),
  muted: svg('<path d="M4 9h4l5-4v14l-5-4H4z"/><path d="M17 9l5 5M22 9l-5 5"/>'),
  music: svg('<path d="M9 18V6l10-2v12"/><circle cx="6.5" cy="18" r="2.5"/><circle cx="16.5" cy="16" r="2.5"/>'),
  plus: svg('<path d="M12 5v14M5 12h14"/>'),
  transition: svg('<path d="M3 6l9 6-9 6zM21 6l-9 6 9 6z"/>'),
};
const has = () => !!(S.tl && S.tl.hasSequence);
const dur = () => has() ? S.tl.duration : 0;
const frameStep = () => 1 / Math.max(1, S.tl ? S.tl.fps : 30);
const clampT = t => has() ? Math.min(Math.max(0, t), Math.max(0, dur() - frameStep())) : 0;
const snapFrame = t => Math.round(t / frameStep()) * frameStep();
const byStart = (a, b) => a.start - b.start;
/** Main-track clips in order. */
const mainClips = () => S.tl ? S.tl.clips.filter(c => c.track === 'V1').sort(byStart) : [];
/** Every clip the timeline shows: all but the main track's own sound, which is drawn inside it. */
const visibleClips = () => S.tl ? S.tl.clips.filter(c => c.track !== 'A1') : [];
const clipById = id => S.tl && (S.tl.clips.find(c => c.id === id) || capClips().find(c => c.id === id));
const isMain = c => c.track === 'V1';
const laneKind = c => c.track[0] === 'A' ? 'audio' : 'video';
const laneNo = id => parseInt(id.slice(1), 10) || 0;
/** The captions, shaped like clips for the timeline (lane `C1`). */
const capClips = () => S.tl && S.tl.captions.exists ? S.tl.captions.items.map(c => ({ ...c, track: 'C1', kind: 'caption', props: {} })) : [];
const isCap = c => c && c.kind === 'caption';
/** Everything on the timeline's lanes: clips and captions. */
const laneThings = () => visibleClips().concat(capClips());
/** A clip's sound: itself for a sound clip, else its linked sound clip, if any. */
const soundOf = c => c.track[0] === 'A' ? c : (c.link ? S.tl.clips.find(a => a.link === c.link && a.track[0] === 'A') : null);
const hasSound = c => !!soundOf(c);
const mediaOf = item => S.tl && S.tl.media.find(m => m.item === item);
const laneOn = id => { const l = S.tl.lanes.find(l => l.id === id); return !l || l.on; };
const geom = c => S.geom.get(c.id) || c;
const content = () => $('#tl-content');
const scroller = () => $('#tl-scroll');
/** The timeline time under a pointer. */
const timeAt = clientX => (clientX - content().getBoundingClientRect().left - PAD) / S.px;
/** A pointer's height inside the lanes. */
const yAt = clientY => clientY - $('#rows').getBoundingClientRect().top;

/* ---------- edits: one at a time, in order ---------- */
let chain = Promise.resolve();
function serial(fn) {
  const p = chain.then(fn, fn);
  chain = p.catch(() => {});
  return p;
}
/** Runs an engine command after any edit before it; `args` may be a function, read when it runs. */
function run(cmd, args = {}) {
  return serial(async () => {
    if (S.playing) await pause();
    try {
      return await invoke(cmd, typeof args === 'function' ? args() : args);
    } catch (e) {
      toast(e, true);
      return null;
    }
  });
}
async function edit(cmd, args) {
  const tl = await run(cmd, args);
  if (tl) apply(tl);
  return tl;
}
/** An edit that makes clips (`{timeline, clips}`): they become the selection. */
async function make(cmd, args) {
  const r = await run(cmd, args);
  if (!r) return null;
  // select what was made, but not a picture's own sound (selecting the picture covers it)
  const made = r.timeline.clips.filter(c => r.clips.includes(c.id));
  S.sel = new Set(made.filter(c => c.track[0] === 'V' || !made.some(v => v.track[0] === 'V' && v.link && v.link === c.link)).map(c => c.id));
  apply(r.timeline);
  return r;
}

/** Redraws everything from a new timeline. */
function apply(tl, { fit = false } = {}) {
  const had = has();
  S.tl = tl;
  const ids = new Set(laneThings().map(c => c.id));
  for (const id of [...S.sel]) if (!ids.has(id)) S.sel.delete(id);
  if (S.cut != null && !mainCuts().some(c => c.from === S.cut)) S.cut = null;
  S.geom.clear();
  S.t = clampT(S.t);
  if (fit || (!had && has())) fitZoom(false);
  $('#empty').hidden = has();
  $('#play-btn').disabled = !has();
  $('#export-btn').disabled = !has();
  $('#ratio').disabled = !has();
  $('#ratio').value = ratioName(tl.width, tl.height);
  $('#tc-total').textContent = fmtFrames(dur());
  renderProject();
  renderMedia();
  renderTrack();
  renderSelection();
  layoutTimeline();
  fitCanvas();
  if (has()) showFrame(S.t); else clearPreview();
  if (tl.project.dirty && has()) saveSoon();
  renderCaptionsPanel();
  renderTransitionsPanel();
  renderMixer();
  filtersSoon();
}

/* ---------- preview ---------- */
function fitCanvas() {
  const stage = $('#stage'), c = $('#preview'), o = $('#overlay');
  if (!has()) { c.style.width = c.style.height = o.style.width = o.style.height = '0'; return; }
  const aw = stage.clientWidth - 28, ah = stage.clientHeight - 28;
  const k = Math.min(aw / S.tl.width, ah / S.tl.height);
  const w = Math.max(1, Math.floor(S.tl.width * k)), h = Math.max(1, Math.floor(S.tl.height * k));
  c.style.width = o.style.width = w + 'px';
  c.style.height = o.style.height = h + 'px';
  drawOverlay();
}
function clearPreview() {
  const c = $('#preview');
  c.getContext('2d').clearRect(0, 0, c.width, c.height);
  drawOverlay();
}
/** Shows a program frame (`{t}`) or a media frame (`{item, t}`, while trimming). Requests never
    pile up: while one renders, only the latest wish is kept. */
/** The player's width in device pixels, and the quality chosen for it. */
const previewWidth = () => Math.min(3840, Math.ceil(parseFloat($('#preview').style.width || '640') * (window.devicePixelRatio || 1)));
const quality = () => $('#quality').value;
async function show(req) {
  if (!has()) return;
  if (S.frameBusy) { S.want = req; return; }
  S.frameBusy = true;
  try {
    const c = $('#preview');
    const maxWidth = previewWidth();
    const buf = req.item != null
      ? await invoke('thumb', { item: req.item, seconds: Math.max(0, req.t), maxWidth })
      : await invoke('frame', { seconds: req.t, maxWidth, quality: quality() });
    const img = toImage(buf);
    if (c.width !== img.width || c.height !== img.height) { c.width = img.width; c.height = img.height; }
    c.getContext('2d').putImageData(img, 0, 0);
    if (req.item == null) drawScope();
  } catch (e) {
    if (req.item == null) toast(e, true);
  } finally {
    S.frameBusy = false;
    if (S.want) { const next = S.want; S.want = null; show(next); }
  }
  drawOverlay();
}
const showFrame = t => show({ t });

/* The selected picture's frame in the player, which a drag moves and the wheel scales */
const visual = c => c && c.props && c.props.x != null;
/** The selected clip the player can move: one picture or text clip under the playhead. */
function stageClip() {
  if (S.sel.size !== 1) return null;
  const c = clipById([...S.sel][0]);
  return visual(c) && c.start <= S.t + 1e-6 && c.end > S.t ? c : null;
}
/** Where a clip sits on the canvas: centre, size and turn (canvas pixels). Text is estimated. */
function boxOf(c) {
  const p = liveClip(c).props, k = (p.scale || 100) / 100;
  let w, h, cy = p.y;
  if (c.kind === 'text' && p.text && p.text.bounds) {
    // the layers' real extent, placed by the clip's position and scale
    const [x0, y0, x1, y1] = p.text.bounds;
    return { x: p.x + k * ((x0 + x1) / 2 - S.tl.width / 2), y: p.y + k * ((y0 + y1) / 2 - S.tl.height / 2), w: k * (x1 - x0) + 16, h: k * (y1 - y0) + 12, r: (p.rotation || 0) * Math.PI / 180 };
  }
  if (c.kind === 'text' && p.text) {
    const lines = (p.text.text || ' ').split('\n');
    const longest = Math.max(1, ...lines.map(l => l.length));
    w = longest * p.text.size * 0.56 * k + 24;
    h = lines.length * p.text.size * 1.2 * k + 16;
    // point text sits on its first baseline at the centre of its frame
    cy = p.y - p.text.size * 0.35 * k + (lines.length - 1) * p.text.size * 0.6 * k;
  } else {
    const m = mediaOf(c.item);
    w = (m && m.width ? m.width : S.tl.width) * k;
    h = (m && m.height ? m.height : S.tl.height) * k;
  }
  return { x: p.x, y: cy, w, h, r: (p.rotation || 0) * Math.PI / 180 };
}
let guides = { x: false, y: false };
function drawOverlay() {
  const o = $('#overlay'), dpr = window.devicePixelRatio || 1;
  const w = parseFloat(o.style.width) || 0, h = parseFloat(o.style.height) || 0;
  if (o.width !== Math.round(w * dpr) || o.height !== Math.round(h * dpr)) { o.width = Math.round(w * dpr); o.height = Math.round(h * dpr); }
  const g = o.getContext('2d');
  g.setTransform(1, 0, 0, 1, 0, 0);
  g.clearRect(0, 0, o.width, o.height);
  if (has()) { g.setTransform(dpr, 0, 0, dpr, 0, 0); drawGuide(g, w, h); g.setTransform(1, 0, 0, 1, 0, 0); }
  const c = has() && !S.playing ? stageClip() : null;
  $('#stage').classList.toggle('grab', !!c);
  if (!c || !w) return;
  const k = w / S.tl.width * dpr, b = boxOf(c);
  g.setTransform(k, 0, 0, k, 0, 0);
  g.strokeStyle = getComputedStyle(document.documentElement).getPropertyValue('--accent').trim() || '#f4c443';
  g.lineWidth = 2 / k * dpr;
  if (guides.x || guides.y) {
    g.save();
    g.setLineDash([6 / k * dpr, 6 / k * dpr]);
    g.beginPath();
    if (guides.x) { g.moveTo(S.tl.width / 2, 0); g.lineTo(S.tl.width / 2, S.tl.height); }
    if (guides.y) { g.moveTo(0, S.tl.height / 2); g.lineTo(S.tl.width, S.tl.height / 2); }
    g.stroke();
    g.restore();
  }
  g.translate(b.x, b.y);
  g.rotate(b.r);
  g.strokeRect(-b.w / 2, -b.h / 2, b.w, b.h);
  g.fillStyle = g.strokeStyle;
  const r = 5 / k * dpr;
  for (const [x, y] of [[-1, -1], [1, -1], [-1, 1], [1, 1]]) g.fillRect(x * b.w / 2 - r, y * b.h / 2 - r, 2 * r, 2 * r);
}
/** The topmost picture or text clip under the playhead whose frame holds the canvas point. */
function clipAtPoint(x, y) {
  const lanes = visibleClips().filter(c => visual(c) && c.start <= S.t + 1e-6 && c.end > S.t && laneOn(c.track));
  lanes.sort((a, b) => laneNo(b.track) - laneNo(a.track));
  for (const c of lanes) {
    const b = boxOf(c), dx = x - b.x, dy = y - b.y;
    const u = dx * Math.cos(-b.r) - dy * Math.sin(-b.r), v = dx * Math.sin(-b.r) + dy * Math.cos(-b.r);
    if (Math.abs(u) <= b.w / 2 && Math.abs(v) <= b.h / 2) return c;
  }
  return null;
}
function stagePointerDown(e) {
  if (!has() || e.button !== 0 || S.playing) return;
  const c0 = $('#preview'), r = c0.getBoundingClientRect();
  if (!r.width) return;
  const k = S.tl.width / r.width;
  const px = (e.clientX - r.left) * k, py = (e.clientY - r.top) * k;
  // a click picks the clip under the pointer; a drag moves the selected one
  let c = stageClip();
  const hit = clipAtPoint(px, py);
  if (hit && (!c || hit.id !== c.id)) { select([hit.id]); c = hit; }
  if (!c) { if (S.sel.size || S.cut != null) select([]); return; }
  e.preventDefault();
  const x0 = liveClip(c).props.x, y0 = liveClip(c).props.y, cx = e.clientX, cy = e.clientY;
  const key = nextKey('move');
  let moved = false;
  follow(e, ev => {
    let x = x0 + (ev.clientX - cx) * k, y = y0 + (ev.clientY - cy) * k;
    if (!moved && Math.hypot(ev.clientX - cx, ev.clientY - cy) < 3) return;
    moved = true;
    // the frame's centre lines pull it in
    const reach = 8 * k;
    guides = { x: S.snap && !ev.shiftKey && Math.abs(x - S.tl.width / 2) < reach, y: S.snap && !ev.shiftKey && Math.abs(y - S.tl.height / 2) < reach };
    if (guides.x) x = S.tl.width / 2;
    if (guides.y) y = S.tl.height / 2;
    queueProps([c.id], { x: Math.round(x), y: Math.round(y) }, key);
  }, () => { guides = { x: false, y: false }; drawOverlay(); }, { scroll: false });
}
let wheelKey = null, wheelTimer = 0;
function stageWheel(e) {
  const c = stageClip();
  if (!c) return;
  e.preventDefault();
  if (!wheelKey) wheelKey = nextKey('scale');
  clearTimeout(wheelTimer);
  wheelTimer = setTimeout(() => { wheelKey = null; }, 500);
  const pending = propQueue.length ? propQueue[propQueue.length - 1].props.scale : undefined;
  const now = pending != null ? pending : c.props.scale;
  queueProps([c.id], { scale: Math.round(clamp(now * Math.exp(-e.deltaY * 0.0015), 1, 2000) * 10) / 10 }, wheelKey);
}

/* ---------- project bar and autosave ---------- */
function projectName() {
  const p = S.tl && S.tl.project;
  if (!p) return 'Untitled';
  if (p.path) return p.path.split(/[\\/]/).pop().replace(/\.fcproj$/i, '');
  return p.name || 'Untitled';
}
function renderProject() {
  $('#project-name').textContent = projectName();
  $('#project-name').title = S.tl.project.path || 'Not saved yet: ClipCut saves it in Documents\\ClipCut after your first edit';
  const p = S.tl.project;
  if (!has()) setSaveState('');
  else if (!p.dirty) setSaveState('Saved');
  renderTools();
}
function setSaveState(text, err = false) {
  const el = $('#save-state');
  el.textContent = text;
  el.classList.toggle('err', err);
}
let saveTimer = 0, saving = null;
function saveSoon() {
  setSaveState('Edited');
  clearTimeout(saveTimer);
  saveTimer = setTimeout(saveNow, SAVE_MS);
}
/** Saves now (the first save gives the project its folder in Documents\ClipCut). */
async function saveNow() {
  clearTimeout(saveTimer);
  saveTimer = 0;
  while (saving) await saving;
  if (!has()) return;
  saving = (async () => {
    setSaveState('Saving…');
    try {
      const p = await invoke('save_project');
      S.tl.project = p;
      setSaveState(p.dirty ? 'Edited' : 'Saved');
      $('#project-name').textContent = projectName();
      $('#project-name').title = p.path || '';
      renderDetails();
    } catch (e) {
      setSaveState('Not saved', true);
      toast(`The project couldn't be saved: ${e}. Check that there is space on the drive and that Documents\\ClipCut can be written to.`, true);
    }
  })();
  await saving;
  saving = null;
}
async function flushSave() { if (saveTimer || saving) await saveNow(); }

function resetView() { S.t = 0; S.sel.clear(); Thumbs.clear(); Peaks.clear(); propsSig = ''; }
async function newProject() {
  await flushSave();
  const tl = await run('new_project');
  if (tl) { resetView(); apply(tl, { fit: true }); toast('New project'); }
}
async function openProject() {
  await flushSave();
  let path;
  try { path = await invoke('pick_project'); } catch (e) { toast(e, true); return; }
  if (!path) return;
  const tl = await run('open_project', { path });
  if (tl) { resetView(); apply(tl, { fit: true }); }
}

/* ---------- the canvas's shape ---------- */
const RATIOS = { '16:9': [16, 9], '9:16': [9, 16], '1:1': [1, 1], '4:5': [4, 5] };
function ratioName(w, h) {
  let best = '16:9', err = Infinity;
  for (const [name, [a, b]] of Object.entries(RATIOS)) {
    const e = Math.abs(Math.log((w / Math.max(1, h)) / (a / b)));
    if (e < err) { err = e; best = name; }
  }
  return best;
}
async function setRatio(name) {
  const [a, b] = RATIOS[name] || RATIOS['16:9'];
  // keep the short side (1080 stays 1080) and make both sides even
  const short = Math.min(S.tl.width, S.tl.height);
  const even = v => Math.max(16, Math.round(v / 2) * 2);
  const [w, h] = a >= b ? [even(short * a / b), even(short)] : [even(short), even(short * b / a)];
  const tl = await edit('set_canvas', { width: w, height: h });
  $('#ratio').blur();
  if (tl) offerRemake();
}

/* ---------- library: media, audio and text ---------- */
async function importClips() {
  let paths;
  try { paths = await invoke('pick_clips'); } catch (e) { toast(e, true); return; }
  if (paths && paths.length) await addPaths(paths);
}
async function addPaths(paths) {
  toast(paths.length > 1 ? `Adding ${paths.length} files…` : 'Adding…');
  const r = await run('add_clips', () => ({ paths, seconds: S.t }));
  if (!r) return;
  apply(r.timeline);
  if (r.errors.length) toast(r.errors.join('\n'), true);
  else $('#toast').hidden = true;
}
/** Audio tab: music and voiceovers go on a sound lane at the playhead. */
async function importSound() {
  let paths;
  try { paths = await invoke('pick_clips'); } catch (e) { toast(e, true); return; }
  if (!paths || !paths.length) return;
  // an empty project starts with the sound (pictures and titles go over it)
  if (!has()) { await addPaths(paths); return; }
  const r = await run('import_media', { paths });
  if (!r) return;
  apply(r.timeline);
  for (const item of r.items) await addToLane(item, S.t);
  if (r.errors.length) toast(r.errors.join('\n'), true);
}
const addToLane = (item, seconds) => make('add_to_lane', { item, seconds });

let mediaSig = '';
const mediaThumbs = new Map();   // item → canvas
function renderMedia() {
  const media = S.tl.media;
  renderProxies();
  const sig = JSON.stringify(media.map(m => [m.item, m.name, m.duration, m.proxy, m.unreadable]));
  if (sig === mediaSig) return;
  mediaSig = sig;
  const list = $('#media-list'), sounds = $('#audio-list');
  list.innerHTML = '';
  sounds.innerHTML = '';
  for (const m of media) {
    const card = document.createElement('div');
    const video = m.kind !== 'audio';
    card.className = 'media-card' + (video ? '' : ' audio');
    card.dataset.item = m.item;
    card.title = m.path;
    card.innerHTML = (video ? '<canvas width="240" height="135"></canvas>' : `<div class="sound">${ICON.music}</div>`)
      + `<button class="add">${ICON.plus}</button>`
      + '<div class="meta"><span></span><span class="dur"></span></div>';
    card.querySelector('.meta span').textContent = m.name;
    if (m.proxy) { const b = document.createElement('span'); b.className = 'proxy-tag'; b.textContent = 'Proxy'; b.title = 'Has a small copy for smooth playback'; card.appendChild(b); }
    if (m.unreadable) { const b = document.createElement('span'); b.className = 'proxy-tag bad'; b.textContent = 'No picture'; b.title = m.unreadable; card.title = `${m.path}\n${m.unreadable}`; card.appendChild(b); }
    card.querySelector('.dur').textContent = fmtLen(m.duration);
    const add = card.querySelector('.add');
    add.title = video ? 'Add to the end of the main track' : 'Add below the main track, at the playhead';
    add.setAttribute('aria-label', `Add ${m.name}`);
    add.addEventListener('click', e => { e.stopPropagation(); video ? appendMedia(m.item) : addSoundAtPlayhead(m.item); });
    card.addEventListener('pointerdown', e => { if (!e.target.closest('.add') && e.button === 0) startMediaDrag(m, e); });
    card.addEventListener('dblclick', () => video ? appendMedia(m.item) : addSoundAtPlayhead(m.item));
    if (video) { paintMediaThumb(m, card.querySelector('canvas')); list.appendChild(card); } else sounds.appendChild(card);
  }
  if (!sounds.children.length) sounds.innerHTML = '<p class="hint">No sound imported yet.</p>';
}
/* Proxies: small copies of the videos for smooth playback (exports use the originals) */
const PX = { job: null };
function renderProxies() {
  const videos = S.tl.media.filter(m => m.kind === 'video');
  $('#proxies').hidden = !videos.length;
  if (!videos.length) return;
  const have = videos.filter(m => m.proxy).length;
  $('#proxy-use').checked = S.tl.proxies;
  $('#proxy-use').disabled = !have;
  $('#proxy-make').disabled = PX.job != null || have === videos.length;
  if (PX.job == null) {
    $('#proxy-note').textContent = have === videos.length ? 'Every video has a proxy.'
      : have ? `${have} of ${videos.length} videos have a proxy.`
      : 'Big or 4K videos play more smoothly from proxies: small copies made in the background.';
  }
}
async function makeProxies() {
  if (PX.job != null) return;
  const missing = S.tl.media.filter(m => m.kind === 'video' && !m.proxy).map(m => m.item);
  try { PX.job = await invoke('make_proxies', { items: missing }); } catch (e) { toast(e, true); return; }
  $('#proxy-bar').hidden = false;
  renderProxies();
  for (;;) {
    await sleep(400);
    let st = null;
    try { st = await invoke('export_status', { job: PX.job }); } catch (_) { /* keep waiting */ }
    if (!st) break;
    $('#proxy-bar').firstChild.style.width = (st.progress * 100).toFixed(1) + '%';
    $('#proxy-note').textContent = st.finished ? '' : `Making proxies… ${Math.floor(st.progress * 100)}%${st.status ? ' · ' + st.status : ''}`;
    if (st.finished) {
      if (st.error) toast(/cancel/i.test(st.error) ? 'The proxies were cancelled.' : `The proxies could not be made: ${st.error}`, true);
      break;
    }
  }
  PX.job = null;
  $('#proxy-bar').hidden = true;
  const tl = await invoke('poll_jobs').catch(() => null);
  if (tl) {
    // made proxies are used straight away
    apply(tl);
    if (!S.tl.proxies) await edit('set_proxies', { on: true });
    toast('Proxies made: the videos now play from them. Exports still use the originals.');
  }
  renderProxies();
}

function addSoundAtPlayhead(item) {
  if (!has()) { edit('start_with_sound', { item }); return; }
  addToLane(item, S.t);
}
async function paintMediaThumb(m, c) {
  let src = mediaThumbs.get(m.item);
  if (!src) {
    try {
      src = toCanvas(toImage(await invoke('thumb', { item: m.item, seconds: Math.min(1, m.duration / 2), maxWidth: 240 })));
      mediaThumbs.set(m.item, src);
    } catch (e) { return; }  // the card keeps its black picture
  }
  drawCover(c.getContext('2d'), src, 0, 0, c.width, c.height);
}
/** Draws `src` filling the box, cropped to keep its shape. */
function drawCover(g, src, x, y, w, h) {
  const k = Math.max(w / src.width, h / src.height);
  const sw = w / k, sh = h / k;
  g.drawImage(src, (src.width - sw) / 2, (src.height - sh) / 2, sw, sh, x, y, w, h);
}
async function appendMedia(item) {
  const tl = await edit('append_media', { item });
  if (tl) scrollToTime(dur());
}

/** Text tab: styles to start from (the same keys as the Properties panel's text settings). */
const TEXT_STYLES = [
  { name: 'Plain', css: '', style: () => ({}) },
  { name: 'Outline', css: '-webkit-text-stroke: 1.2px #000', style: () => ({ stroke: true, strokeColor: '#000000', strokeWidth: 8 }) },
  { name: 'Box', span: 'background: rgba(0,0,0,.8)', style: () => ({ background: true, backgroundColor: '#000000' }) },
  { name: 'Shadow', css: 'text-shadow: 2px 3px 4px #000', style: () => ({ shadow: true }) },
  { name: 'Caption', css: 'color: #ffd400; -webkit-text-stroke: 1.2px #000', style: () => ({ color: '#ffd400', stroke: true, strokeColor: '#000000', strokeWidth: 10, bold: true }) },
  { name: 'Title', css: 'font-size: 26px; font-weight: 900', style: () => ({ bold: true, size: Math.round(S.tl.height * 0.12) }) },
  { name: 'Strap', span: 'background: #c8102e', style: () => ({ background: true, backgroundColor: '#c8102e', bold: true }) },
  { name: 'Soft', css: 'color: #f4f0e6; font-weight: 400; font-family: Georgia, serif', style: () => ({ font: 'Noto Serif', color: '#f4f0e6' }) },
];
function renderTextStyles() {
  const box = $('#text-styles');
  for (const s of TEXT_STYLES) {
    const b = document.createElement('button');
    b.className = 'text-style';
    b.title = `Add ${s.name.toLowerCase()} text at the playhead`;
    b.innerHTML = `<span style="${s.span || ''}">${s.name}</span>`;
    if (s.css) b.querySelector('span').style.cssText += ';' + s.css;
    b.addEventListener('click', () => addText(s));
    box.appendChild(b);
  }
}
async function addText(s) {
  if (!has()) { toast('Add a video first: text goes over it.'); return; }
  const r = await make('add_text', () => ({ seconds: S.t, duration: 3, text: 'Add text', style: s.style() }));
  if (r) { const ta = $('#props textarea'); if (ta) { ta.focus(); ta.select(); } }
}
function showTab(id) {
  for (const t of ['media', 'audio', 'text', 'transitions', 'filters', 'mixer', 'captions']) {
    $(`#tab-${t}`).setAttribute('aria-selected', String(t === id));
    $(`#panel-${t}`).hidden = t !== id;
  }
  if (id === 'captions') openCaptionsPanel();
  if (id === 'transitions') openTransitionsPanel();
  if (id === 'filters') renderFiltersPanel();
  if (id === 'mixer') { MX.sig = ''; renderMixer(); }
  if (id === 'text') renderTitles();
}

/* ---------- captions: auto captions, their look, cleaning up the speech, the transcript ---------- */
const CAP = { models: null, presets: null, busy: false };
async function openCaptionsPanel() {
  if (!CAP.models) {
    try {
      const r = await invoke('speech_models');
      CAP.models = r.models;
      const sel = $('#cap-model');
      sel.innerHTML = '';
      const names = { 'whisper-tiny': 'Fast', 'whisper-base': 'Balanced', 'whisper-small': 'Best (slow)' };
      for (const m of r.models) {
        const o = document.createElement('option');
        o.value = m.id;
        o.textContent = `${names[m.id] || m.name} · ${Math.round(m.size / 1e6)} MB${m.installed ? ', ready' : ''}`;
        sel.appendChild(o);
      }
      let want = 'whisper-base';
      try { want = localStorage.getItem('clipcut.model') || want; } catch (_) {}
      if ([...sel.options].some(o => o.value === want)) sel.value = want;
    } catch (e) { toast(e, true); }
    try { const l = localStorage.getItem('clipcut.lang'); if (l) $('#cap-lang').value = l; } catch (_) {}
  }
  if (!CAP.presets) {
    try {
      CAP.presets = await invoke('caption_presets');
      const box = $('#cap-styles');
      for (const p of CAP.presets) {
        const b = document.createElement('button');
        b.className = 'text-style cap';
        b.title = `${p.name} captions`;
        const st = p.style;
        b.innerHTML = `<span>${p.name}</span>`;
        const span = b.querySelector('span');
        span.style.color = st.color;
        if (st.background) span.style.background = st.backgroundColor;
        if (st.outline) span.style.webkitTextStroke = `1px ${st.outlineColor}`;
        if (st.font === 'Noto Serif') span.style.fontFamily = 'Georgia, serif';
        b.addEventListener('click', () => edit('set_caption_style', { style: st }));
        box.appendChild(b);
      }
    } catch (e) { /* the presets stay hidden */ }
  }
  renderCaptionsPanel();
  loadWords();
}
function capNote() {
  const m = CAP.models && CAP.models.find(x => x.id === $('#cap-model').value);
  const c = S.tl && S.tl.captions;
  let note = 'The speech model downloads once and runs on this computer: nothing is uploaded.';
  if (m && !m.installed) note = `The first run downloads this speech model (${Math.round(m.size / 1e6)} MB, once). It runs on this computer: nothing is uploaded.`;
  if (c && c.exists && c.untranscribed) note = `${c.untranscribed} clip${c.untranscribed > 1 ? 's' : ''} on the main track ${c.untranscribed > 1 ? "aren't" : "isn't"} captioned yet: press Auto captions to add ${c.untranscribed > 1 ? 'them' : 'it'}.`;
  else if (c && c.exists) note = 'Captions follow the speech when you cut, trim or move clips. Press Auto captions again to lay them out afresh.';
  $('#cap-note').textContent = note;
}
function renderCaptionsPanel() {
  if (!S.tl) return;
  const c = S.tl.captions;
  $('#cap-tools').hidden = !c.exists;
  $('#cap-on').checked = c.on;
  $('#cap-go').disabled = CAP.busy || !has();
  $('#cap-go').textContent = c.exists ? 'Auto captions again' : 'Auto captions';
  capNote();
  if (!$('#panel-captions').hidden) loadWordsSoon();
}
async function autoCaptions() {
  if (!has() || CAP.busy) return;
  CAP.busy = true;
  $('#cap-go').disabled = true;
  $('#cap-progress').hidden = false;
  $('#cap-bar').style.width = '0';
  $('#cap-status').textContent = 'Starting…';
  if (S.playing) await pause();
  // again: lay the captions out afresh from all the speech (an existing transcript is reused)
  const again = false;
  try {
    const tl = await invoke('auto_captions', { model: $('#cap-model').value, language: $('#cap-lang').value, again });
    apply(tl);
    toast(`${tl.captions.items.length} captions made`);
    CAP.models = null;  // a model may have been downloaded
    openCaptionsPanel();
  } catch (e) {
    if (/cancel/i.test(String(e))) toast('Captions cancelled: nothing was changed');
    else toast(e, true);
  } finally {
    CAP.busy = false;
    $('#cap-progress').hidden = true;
    renderCaptionsPanel();
  }
}
async function removeFillers() {
  const r = await run('remove_fillers');
  if (!r) return;
  apply(r.timeline);
  toast(r.removed ? `${r.removed} filler word${r.removed > 1 ? 's' : ''} cut` : 'No filler words found');
}
async function removePauses() {
  const r = await run('remove_pauses', { minSeconds: 0.5, keepSeconds: 0.15 });
  if (!r) return;
  apply(r.timeline);
  toast(r.removed ? `${r.removed} pause${r.removed > 1 ? 's' : ''} shortened, ${r.seconds.toFixed(1)} s saved` : 'No long pauses found');
}
async function saveSubtitles() {
  let path;
  try { path = await invoke('pick_captions_path', { name: projectName() }); } catch (e) { toast(e, true); return; }
  if (!path) return;
  try {
    const n = await invoke('export_captions', { path });
    toast(`${n} captions saved`);
  } catch (e) { toast(e, true); }
}

/* The transcript: the main track's words, as paragraphs; click to go, drag or Shift+click to
   select, Delete to cut */
const W = { words: [], spans: [], sel: null, anchor: null, now: -1 };
let wordsTimer = 0;
function loadWordsSoon() { clearTimeout(wordsTimer); wordsTimer = setTimeout(loadWords, 120); }
async function loadWords() {
  if ($('#panel-captions').hidden || !has()) return;
  try { W.words = await invoke('words'); } catch (e) { W.words = []; }
  W.sel = null;
  renderTranscript();
}
function renderTranscript() {
  const box = $('#transcript');
  box.innerHTML = '';
  W.spans = [];
  W.now = -1;
  if (!W.words.length) { box.innerHTML = '<p class="empty">No transcript yet: press Auto captions.</p>'; return; }
  let p = null, last = null;
  W.words.forEach((w, i) => {
    if (!p || w.start - last.end > 1.2 || w.speaker !== last.speaker) {
      p = document.createElement('p');
      if (w.speaker && (!last || w.speaker !== last.speaker)) { const who = document.createElement('span'); who.className = 'who'; who.textContent = w.speaker; p.appendChild(who); }
      box.appendChild(p);
    }
    const sp = document.createElement('span');
    sp.className = 'w';
    sp.dataset.i = i;
    sp.textContent = w.text;
    p.appendChild(sp);
    p.appendChild(document.createTextNode(' '));
    W.spans.push(sp);
    last = w;
  });
  markWord();
}
function paintWordSel() {
  W.spans.forEach((sp, i) => sp.classList.toggle('sel', !!W.sel && i >= W.sel[0] && i <= W.sel[1]));
}
function wireTranscript() {
  const box = $('#transcript');
  let dragging = false;
  box.addEventListener('mousedown', e => {
    const sp = e.target.closest('span.w');
    if (!sp || e.button !== 0) return;
    e.preventDefault();
    box.focus();
    const i = +sp.dataset.i;
    if (e.shiftKey && W.anchor != null) W.sel = [Math.min(W.anchor, i), Math.max(W.anchor, i)];
    else { W.anchor = i; W.sel = null; seek(W.words[i].start); scrollToTime(W.words[i].start); }
    dragging = true;
    paintWordSel();
  });
  box.addEventListener('mouseover', e => {
    if (!dragging || W.anchor == null) return;
    const sp = e.target.closest('span.w');
    if (!sp) return;
    const i = +sp.dataset.i;
    W.sel = [Math.min(W.anchor, i), Math.max(W.anchor, i)];
    paintWordSel();
  });
  window.addEventListener('mouseup', () => { dragging = false; });
}
async function cutSelectedWords() {
  if (!W.sel) return;
  const [a, b] = W.sel;
  W.sel = null;
  const tl = await edit('cut_words', { from: a, to: b });
  if (tl) toast(a === b ? 'Word cut' : `${b - a + 1} words cut`);
}
/** Lights the word under the playhead (and keeps it in view while playing). */
function markWord() {
  if (!W.spans.length || $('#panel-captions').hidden) return;
  let lo = 0, hi = W.words.length - 1, at = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (W.words[mid].start <= S.t + 1e-6) { at = mid; lo = mid + 1; } else hi = mid - 1;
  }
  if (at >= 0 && W.words[at].end < S.t - 0.3) at = -1;
  if (at === W.now) return;
  if (W.now >= 0 && W.spans[W.now]) W.spans[W.now].classList.remove('now');
  W.now = at;
  if (at >= 0 && W.spans[at]) {
    W.spans[at].classList.add('now');
    if (S.playing) W.spans[at].scrollIntoView({ block: 'nearest' });
  }
}

/* ---------- Graphics: Clip Studio over the editor; its graphics come in as clips ---------- */
const TOOL_NAMES = { ul: 'Underline', mp: 'Map', sp: 'Split screen', sz: 'Scroll zoom', tl: 'Timeline', nm: 'Numbers', au: 'Audiogram', lt: 'Lower third' };
const GR = { loaded: null, batch: false };
/** Graphics covers the editor: while it shows, the editor underneath can't take focus or clicks. */
function showGraphicsView(on) {
  $('#graphics-view').hidden = !on;
  $('.app').inert = on;
}
const graphicsWin = () => $('#graphics-frame').contentWindow;
/* What Clip Studio calls (see "Inside ClipCut" in graphics/js/core.js): it renders with its own draw
   functions and hands each frame here; the frames go to a folder beside the project, then the timeline. */
window.__clipcutHost = {
  get aspect() { return S.tl && S.tl.hasSequence ? ratioName(S.tl.width, S.tl.height) : '16:9'; },
  begin: info => invoke('graphic_begin', { name: info.name || 'Graphic' }),
  frame: (job, i, bytes, ext) => invoke('graphic_frame', bytes, { headers: { job: String(job), index: String(i), ext } }),
  audio: (job, bytes) => invoke('graphic_audio', bytes, { headers: { job: String(job) } }),
  async finish(job, info) {
    const replace = typeof info.edit === 'number' && clipById(info.edit) ? info.edit : null;
    const r = await run('graphic_finish', { job, info, seconds: S.t, replace });
    if (!r) throw new Error('The graphic could not be added to the video.');
    if (GR.batch) { apply(r.timeline); return; }
    S.sel = new Set(r.clips.filter(id => r.timeline.clips.some(c => c.id === id && c.track[0] === 'V')));
    apply(r.timeline);
    closeGraphics();
    toast(replace != null ? 'Graphic updated' : 'Graphic added at the playhead');
  },
  abort: job => { invoke('graphic_abort', { job }).catch(() => {}); },
  // a thumbnail, a rendered video or presets Graphics would download: the Save dialog, then the file
  async save(blob, name) {
    let path;
    try { path = await invoke('save_graphic_file', { name }); } catch (e) { toast(e, true); return null; }
    if (!path) return null;
    try { await invoke('write_graphic_file', new Uint8Array(await blob.arrayBuffer())); } catch (e) { toast(e, true); return null; }
    toast(`Saved ${path.split(/[\\/]/).pop()}`);
    return path;
  },
};
function loadGraphics() {
  if (!GR.loaded) {
    GR.loaded = new Promise((resolve, reject) => {
      const f = $('#graphics-frame');
      f.addEventListener('load', () => (graphicsWin().CS && graphicsWin().CS.hosted ? resolve() : reject(new Error('Graphics did not load.'))), { once: true });
      f.src = 'graphics/index.html';
    });
    GR.loaded.catch(() => { GR.loaded = null; });
  }
  return GR.loaded;
}
/** Opens the Graphics view: for a new graphic, or (`edit`, a clip) to change the graphic it is. */
async function openGraphics(edit = null) {
  if (S.playing) await pause();
  let made = null;
  if (edit != null) {
    try { made = await invoke('graphic_info', { clip: edit }); } catch (e) { toast(e, true); return; }
  }
  showGraphicsView(true);
  try { await loadGraphics(); } catch (e) { showGraphicsView(false); toast(e, true); return; }
  const cs = graphicsWin().CS;
  if (made && made.tool) cs.hosted.open(made.tool, made.state, edit);
  else cs.hosted.open(null);
  // the video's shape, even for a graphic made when it had another
  cs.hosted.setAspect(window.__clipcutHost.aspect);
  showBackdrop(cs, edit);
  $('#graphics-note').innerHTML = made
    ? `Changing <b>${TOOL_NAMES[made.tool] || 'a graphic'}</b>. <b>Update in video</b> puts the new version in its place. Photos, videos and sound it used must be added again.`
    : 'Made with Clip Studio. <b>Add to video</b> puts the graphic on a lane at the playhead.';
  $('#graphics-frame').focus();
}
/** Graphics shows the video under what is being made: the frame at the playhead (or, for a graphic
 *  being changed, inside it), without that graphic. */
async function showBackdrop(cs, edit) {
  if (!cs.hosted.setBackdrop || !has()) return;
  const c = edit != null ? clipById(edit) : null;
  const t = c && (S.t < c.start || S.t >= c.end) ? c.start + Math.min(1, (c.end - c.start) / 2) : S.t;
  try {
    const img = toImage(await invoke('graphic_backdrop', { seconds: t, maxWidth: 960, hide: c ? [c.id] : [] }));
    const cv = toCanvas(img);
    await cs.hosted.setBackdrop(cv.toDataURL('image/jpeg', 0.9));
  } catch (_) { /* designing without it */ }
}
/* After the video changes shape: its graphics, made for the old one, made again for the new one */
const shapeOf = (w, h) => ratioName(w, h);
function graphicsInOtherShape() {
  const want = shapeOf(S.tl.width, S.tl.height);
  return S.tl.clips.filter(c => c.graphic && c.track[0] === 'V').filter(c => {
    const m = mediaOf(c.item);
    return m && m.width && m.height && shapeOf(m.width, m.height) !== want;
  });
}
async function offerRemake() {
  const list = graphicsInOtherShape();
  if (!list.length) return;
  const shape = shapeOf(S.tl.width, S.tl.height);
  const n = list.length;
  $('#remake-text').textContent = `${n === 1 ? 'A graphic was' : `${n} graphics were`} made for another shape. Make ${n === 1 ? 'it' : 'them'} again for ${shape}, in the same places? Graphics renders ${n === 1 ? 'it' : 'each one'} again; photos and videos ${n === 1 ? 'it' : 'they'} used must be added again.`;
  $('#remake-go').textContent = `Make for ${shape}`;
  $('#remake-skip').textContent = n === 1 ? 'Keep it as it is' : 'Keep them as they are';
  const go = await new Promise(done => {
    $('#remake-go').onclick = () => { $('#remake-dlg').close(); done(true); };
    $('#remake-skip').onclick = () => { $('#remake-dlg').close(); done(false); };
    $('#remake-dlg').oncancel = () => done(false);
    $('#remake-dlg').showModal();
  });
  if (go) await remakeGraphics(list.map(c => c.id));
}
async function remakeGraphics(ids) {
  if (S.playing) await pause();
  showGraphicsView(true);
  try { await loadGraphics(); } catch (e) { showGraphicsView(false); toast(e, true); return; }
  const cs = graphicsWin().CS;
  GR.batch = true;
  let done = 0;
  try {
    for (const [k, id] of ids.entries()) {
      let made;
      try { made = await invoke('graphic_info', { clip: id }); } catch (_) { continue; }
      if (!made || !made.tool || !clipById(id)) continue;
      $('#graphics-note').innerHTML = `Making graphics for ${shapeOf(S.tl.width, S.tl.height)}: <b>${k + 1} of ${ids.length}</b> (${TOOL_NAMES[made.tool] || 'graphic'}).`;
      cs.hosted.open(made.tool, made.state, id);
      cs.hosted.setAspect(window.__clipcutHost.aspect);
      const status = await cs.hosted.add(made.tool);
      if (status === 'done') done += 1;
      else if (status !== 'busy') break;
    }
  } finally {
    GR.batch = false;
    closeGraphics();
  }
  toast(done === ids.length ? `${done === 1 ? 'The graphic was' : `All ${done} graphics were`} made for the new shape.` : `${done} of ${ids.length} graphics were made for the new shape. Open the others with Edit in Graphics.`, done < ids.length);
}
function closeGraphics() {
  showGraphicsView(false);
  // a graphic being changed and left unfinished: the next one is new
  try { graphicsWin().CS?.hosted?.open(null); } catch (_) { /* not loaded */ }
  $('#tab-graphics').blur();
}

/* The player's overlay: what TikTok, Reels or Shorts cover (9:16), or the safe zones. Never exported. */
let guideKind = 'none';
try { guideKind = localStorage.getItem('clipcut.guide') || 'none'; } catch (_) {}
function drawGuide(g, w, h) {
  if (guideKind === 'none' || !w || !h) return;
  const tall = S.tl && ratioName(S.tl.width, S.tl.height) === '9:16';
  if (guideKind !== 'safe' && tall && window.CS && window.CS.drawPlatformUI) { window.CS.drawPlatformUI(g, w, h, guideKind); return; }
  g.save();
  g.lineWidth = Math.max(1, w / 640);
  g.setLineDash([6, 5]);
  g.strokeStyle = 'rgba(244,196,67,0.85)';
  g.strokeRect(w * 0.05, h * 0.05, w * 0.9, h * 0.9);
  g.strokeStyle = 'rgba(255,255,255,0.45)';
  g.strokeRect(w * 0.1, h * 0.1, w * 0.8, h * 0.8);
  g.restore();
  if (guideKind !== 'safe' && !tall) {
    g.save();
    g.font = `600 ${Math.max(11, w / 60)}px system-ui, sans-serif`;
    g.fillStyle = 'rgba(244,196,67,0.9)';
    g.fillText('App overlays are for 9:16 videos', w * 0.05 + 6, h * 0.05 + Math.max(14, w / 45));
    g.restore();
  }
}

/* ---------- transitions: one per cut on the main track ---------- */
const TR = { presets: null, frames: null, w: 0, h: 0, n: 0, len: 0.5 };
try { const v = +localStorage.getItem('clipcut.trlen'); if (v > 0) TR.len = v; } catch (_) {}
const transitionAt = from => (S.tl && from != null ? S.tl.transitions.find(t => t.from === from) : null) || null;
/** The main track's cuts, left to right: {from, to, at}. */
function mainCuts() {
  const m = mainClips(), out = [];
  for (let k = 0; k + 1 < m.length; k++) if (Math.abs(m[k].end - m[k + 1].start) < 1e-6) out.push({ from: m[k].id, to: m[k + 1].id, at: m[k + 1].start });
  return out;
}
function selectCut(from) {
  S.sel.clear();
  S.cut = from;
  renderSelection();
}
/** The cut a tile goes on: the selected one, else the one nearest the playhead. */
function targetCut() {
  const cs = mainCuts();
  if (S.cut != null && cs.some(c => c.from === S.cut)) return S.cut;
  if (!cs.length) return null;
  return cs.reduce((a, b) => Math.abs(b.at - S.t) < Math.abs(a.at - S.t) ? b : a).from;
}
async function openTransitionsPanel() {
  if (!TR.presets) {
    try { TR.presets = await invoke('transition_presets'); } catch (e) { toast(e, true); return; }
    const grid = $('#tr-grid');
    for (const p of TR.presets) {
      const b = document.createElement('button');
      b.className = 'tr-tile';
      b.dataset.id = p.id;
      b.title = `Put ${p.name} on the cut`;
      b.innerHTML = '<canvas width="160" height="90"></canvas><span></span>';
      b.querySelector('span').textContent = p.name;
      b.addEventListener('click', () => pickTransition(p.id));
      b.addEventListener('pointerenter', () => animateTile(b, true));
      b.addEventListener('pointerleave', () => animateTile(b, false));
      grid.appendChild(b);
    }
    try {
      const buf = await invoke('transition_previews', { w: 160, h: 90, frames: 16 });
      const dv = new DataView(buf);
      [TR.w, TR.h, TR.n] = [dv.getUint32(0, true), dv.getUint32(4, true), dv.getUint32(8, true)];
      TR.frames = buf;
      for (const b of grid.children) paintTile(b, restFrame());
    } catch (e) { /* the tiles stay plain */ }
  }
  renderTransitionsPanel();
}
/** The frame a tile shows at rest: a third of the way, where every transition shows itself. */
const restFrame = () => Math.round((TR.n - 1) * 0.3);
function paintTile(b, k) {
  if (!TR.frames) return;
  const i = TR.presets.findIndex(p => p.id === b.dataset.id), size = TR.w * TR.h * 4;
  const c = b.querySelector('canvas');
  if (c.width !== TR.w || c.height !== TR.h) { c.width = TR.w; c.height = TR.h; }
  c.getContext('2d').putImageData(new ImageData(new Uint8ClampedArray(TR.frames, 12 + (i * TR.n + k) * size, size), TR.w, TR.h), 0, 0);
}
/** Plays a tile's preview while the pointer is on it: through, then a short hold at each end. */
function animateTile(b, on) {
  cancelAnimationFrame(b.raf || 0);
  if (!TR.frames) return;
  if (!on) { paintTile(b, restFrame()); return; }
  const t0 = performance.now();
  const step = now => {
    const cyc = ((now - t0) / 1500) % 1;
    paintTile(b, Math.round(clamp((cyc - 0.15) / 0.6, 0, 1) * (TR.n - 1)));
    b.raf = requestAnimationFrame(step);
  };
  b.raf = requestAnimationFrame(step);
}
function renderTransitionsPanel() {
  if (!S.tl || !TR.presets) return;
  const from = targetCut(), t = transitionAt(from);
  for (const b of $('#tr-grid').children) b.setAttribute('aria-pressed', String(!!t && t.preset === b.dataset.id));
  const cs = mainCuts();
  $('#tr-target').textContent = !cs.length ? 'Transitions go between clips: add a second clip to the main track.'
    : S.cut == null ? 'Click the square between two clips on the main track, or pick one here for the cut nearest the playhead.'
    : t ? `This cut has ${t.name}: pick another to change it.` : 'Pick a transition for the selected cut.';
  $('#tr-all').disabled = !cs.length;
}
async function pickTransition(id) {
  const from = targetCut();
  if (from == null) { toast('Add a second clip to the main track first: transitions go between clips.'); return; }
  const tl = await edit('set_transition', () => ({ from, preset: id, seconds: transitionAt(from) ? null : TR.len }));
  if (!tl) return;
  S.cut = from;
  renderSelection();
  // show it half way through
  const t = transitionAt(from);
  if (t) { seek(t.at); scrollToTime(t.at); }
}
async function transitionEverywhere() {
  const t = transitionAt(targetCut());
  const preset = t && t.preset !== 'other' ? t.preset : 'mix';
  const tl = await edit('transition_everywhere', { preset, seconds: t ? t.end - t.start : TR.len });
  if (tl) toast(`${tl.transitions.length} cut${tl.transitions.length === 1 ? '' : 's'} now have ${(TR.presets || []).find(p => p.id === preset)?.name || 'a transition'}`);
}
/* A dragged length slider sends its latest value; each drag is one undo step */
let trLenWant = null, trLenBusy = false;
async function queueTransitionLength(from, seconds, merge) {
  trLenWant = { from, seconds, merge };
  if (trLenBusy) return;
  trLenBusy = true;
  try {
    while (trLenWant) {
      const w = trLenWant;
      trLenWant = null;
      await edit('set_transition_length', w);
    }
  } finally {
    trLenBusy = false;
  }
}
/** The selected cut's transition: its length, and what to do with it. */
function transitionControls(box, t) {
  if (!t) {
    box.innerHTML = '<p class="hint">A plain cut. Pick a transition for it in the Transitions tab.</p>';
    box.appendChild(buttons([['Transitions tab', 'Show the transitions', () => showTab('transitions')]]));
    return;
  }
  const row = document.createElement('label');
  row.className = 'prop';
  row.innerHTML = `<span>Length</span><input type="range" min="0.1" max="${Math.max(0.1, t.max).toFixed(2)}" step="0.05"><input type="number" step="0.1" title="seconds">`;
  const [range, num] = row.querySelectorAll('input');
  range.id = 'prop-transition-length';
  let k = nextKey('trlen');
  const send = (v, merge) => {
    num.value = (+v).toFixed(2);
    TR.len = +v;
    try { localStorage.setItem('clipcut.trlen', String(TR.len)); } catch (_) {}
    queueTransitionLength(t.from, +v, merge);
  };
  range.addEventListener('input', () => send(range.value, k));
  range.addEventListener('change', () => { k = nextKey('trlen'); range.blur(); });
  num.addEventListener('change', () => { if (+num.value > 0) { range.value = num.value; send(num.value, null); } });
  const read = () => { const x = transitionAt(t.from); return x ? x.end - x.start : 0; };
  controls.push({ el: range, read }, { el: num, read, write: (el, v) => { el.value = v.toFixed(2); } });
  box.appendChild(row);
  box.appendChild(buttons([
    ['Remove', 'Back to a plain cut (Delete)', () => edit('remove_transitions', { from: [t.from] })],
    ['Apply to every cut', `Put ${t.name} on every cut`, transitionEverywhere],
    ['Change', 'Pick another in the Transitions tab', () => showTab('transitions')],
  ]));
}

/* ---------- colour: Adjust in Properties, the Filters tab, the scopes ---------- */
// name, label, range, step, neutral (clipcut-core's colour::ADJUST)
const ADJ = [
  ['exposure', 'Exposure', -5, 5, 0.05, 0], ['contrast', 'Contrast', -100, 100, 1, 0], ['highlights', 'Highlights', -100, 100, 1, 0],
  ['shadows', 'Shadows', -100, 100, 1, 0], ['whites', 'Whites', -100, 100, 1, 0], ['blacks', 'Blacks', -100, 100, 1, 0],
  ['temperature', 'Temperature', -100, 100, 1, 0], ['tint', 'Tint', -100, 100, 1, 0], ['saturation', 'Saturation', 0, 200, 1, 100],
  ['vibrance', 'Vibrance', -100, 100, 1, 0], ['fade', 'Fade', 0, 100, 1, 0], ['sharpen', 'Sharpen', -100, 100, 1, 0], ['vignette', 'Vignette', -5, 5, 0.05, 0],
];
/* A dragged adjustment sends its latest value; each drag is one undo step */
let colWant = null, colBusy = false;
async function queueColour(ids, values, merge) {
  if (colWant && colWant.merge === merge && String(colWant.clips) === String(ids)) Object.assign(colWant.values, values);
  else colWant = { clips: ids, values: { ...values }, merge };
  if (colBusy) return;
  colBusy = true;
  try {
    while (colWant) {
      const w = colWant;
      colWant = null;
      await edit('set_colour', w);
    }
  } finally {
    colBusy = false;
  }
}
/** The Adjust section: every adjustment, Reset and Apply to all clips. Folds away. */
function adjustControls(box, ids) {
  const d = document.createElement('details');
  d.className = 'adjust';
  let open = false;
  try { open = localStorage.getItem('clipcut.adjust') === 'open'; } catch (_) {}
  d.open = open;
  d.innerHTML = '<summary>Adjust</summary>';
  d.addEventListener('toggle', () => { try { localStorage.setItem('clipcut.adjust', d.open ? 'open' : ''); } catch (_) {} });
  for (const [key, label, min, max, step, neutral] of ADJ) {
    const row = document.createElement('label');
    row.className = 'prop';
    row.innerHTML = `<span title="Double-click to reset">${label}</span><input type="range" min="${min}" max="${max}" step="${step}"><input type="number" step="${step}">`;
    const [range, num] = row.querySelectorAll('input');
    range.id = `adj-${key}`;
    num.dataset.dp = step < 1 ? '2' : '0';
    let k = nextKey(key);
    const read = c => (c.props.colour ? c.props.colour.values[key] : neutral);
    const send = (v, merge) => { num.value = v; queueColour(ids, { [key]: +v }, merge); };
    range.addEventListener('input', () => send(range.value, k));
    range.addEventListener('change', () => { k = nextKey(key); range.blur(); });
    num.addEventListener('change', () => { if (Number.isFinite(+num.value)) { range.value = num.value; send(+num.value, null); } });
    row.firstChild.addEventListener('dblclick', e => { e.preventDefault(); range.value = neutral; send(neutral, null); });
    controls.push({ el: range, read }, { el: num, read });
    d.appendChild(row);
  }
  d.appendChild(buttons([
    ['Reset', 'Take every adjustment and filter off this clip', () => edit('reset_colour', { clips: ids })],
    ['Apply to all clips', "Give every clip on the main track this clip's colour", async () => {
      const tl = await edit('colour_everywhere', { clip: ids[0] });
      if (tl) toast("Every clip on the main track has this clip's colour");
    }],
  ]));
  box.appendChild(d);
}

/* The Filters tab: the clip's picture under each of the engine's looks */
const FL = { looks: null, sig: '', timer: 0 };
const pictureClip = c => c && !isCap(c) && c.track[0] === 'V' && c.kind !== 'text';
/** The clip a filter goes on: the selected picture clip, else the main-track clip under the playhead. */
function filterTarget() {
  const sel = selectedClips();
  if (sel.length === 1) return pictureClip(sel[0]) ? sel[0] : null;
  return mainClips().find(c => S.t >= c.start && S.t < c.end) || null;
}
function filtersSoon() { clearTimeout(FL.timer); FL.timer = setTimeout(renderFiltersPanel, 150); }
async function renderFiltersPanel() {
  if ($('#panel-filters').hidden || !S.tl) return;
  if (!FL.looks) { try { FL.looks = await invoke('looks'); } catch (e) { toast(e, true); return; } }
  const c = filterTarget(), grid = $('#fl-grid');
  const col = c && c.props.colour;
  $('#fl-target').textContent = !has() ? 'Add a video first.' : !c ? 'Select a picture clip to give it a filter.'
    : col && col.look ? `${c.name} has ${col.look}. Pick another, or None to take it off.` : `Pick a filter for ${c.name}.`;
  $('#fl-strength-row').hidden = !(col && col.look);
  if (col) { $('#fl-strength').value = col.intensity; $('#fl-strength-n').value = Math.round(col.intensity); }
  if (!c) { grid.innerHTML = ''; FL.sig = ''; return; }
  const names = ['None', ...FL.looks];
  const sig = `${c.id}:${Math.round(S.t * 10)}:${c.start}:${c.sourceIn}`;
  if (sig !== FL.sig || grid.children.length !== names.length) {
    FL.sig = sig;
    let buf;
    try { buf = await invoke('look_previews', { clip: c.id, seconds: S.t, width: 160 }); } catch (e) { return; }
    const dv = new DataView(buf), n = dv.getUint32(0, true);
    let at = 4;
    grid.innerHTML = '';
    for (let i = 0; i < n && i < names.length; i++) {
      const w = dv.getUint32(at, true), h = dv.getUint32(at + 4, true);
      const img = new ImageData(new Uint8ClampedArray(buf, at + 8, w * h * 4), w, h);
      at += 8 + w * h * 4;
      const b = document.createElement('button');
      b.className = 'tr-tile';
      b.dataset.look = i === 0 ? '' : names[i];
      b.title = i === 0 ? 'No filter' : `Give the clip ${names[i]}`;
      b.innerHTML = '<canvas></canvas><span></span>';
      b.querySelector('span').textContent = names[i];
      const cv = b.querySelector('canvas');
      cv.width = w; cv.height = h;
      cv.getContext('2d').putImageData(img, 0, 0);
      b.addEventListener('click', () => {
        const t = filterTarget();
        if (!t) return;
        edit('set_look', { clips: [t.id], look: b.dataset.look || null, intensity: +$('#fl-strength').value || 100 });
      });
      grid.appendChild(b);
    }
  }
  for (const b of grid.children) b.setAttribute('aria-pressed', String((b.dataset.look || null) === ((col && col.look) || null)));
}
let flKey = 'strength:0';   // a new key each drag (nextKey is defined further down)
function sendStrength(v, merge) {
  const t = filterTarget();
  const col = t && t.props.colour;
  if (!col || !col.look) return;
  $('#fl-strength-n').value = Math.round(v);
  edit('set_look', { clips: [t.id], look: col.look, intensity: +v, merge });
}

/* Scopes of the frame in the player, above Properties */
const SC = { on: false, busy: false, want: false };
async function drawScope() {
  if (!SC.on || !has()) return;
  if (SC.busy) { SC.want = true; return; }
  SC.busy = true;
  try {
    const img = toImage(await invoke('scope', { seconds: S.t, kind: $('#scope-kind').value }));
    const c = $('#scope-canvas');
    if (c.width !== img.width || c.height !== img.height) { c.width = img.width; c.height = img.height; }
    c.getContext('2d').putImageData(img, 0, 0);
  } catch (_) { /* nothing to measure */ } finally {
    SC.busy = false;
    if (SC.want) { SC.want = false; drawScope(); }
  }
}
function setScopes(on) {
  SC.on = on;
  $('#scopes').hidden = !on;
  $('#scopes-btn').setAttribute('aria-pressed', String(on));
  try { localStorage.setItem('clipcut.scopes', on ? $('#scope-kind').value : ''); } catch (_) {}
  if (on) drawScope();
}

/* ---------- the mixer: each sound lane's volume, pan, mute and solo, the Mix, and meters ---------- */
const MX = { sig: '', level: new Map(), at: 0, raf: 0 };
const METER_LO = -48, METER_HI = 6;   // dB across a meter
// the main track's sound, and every sound lane with something on it
const soundLanes = () => S.tl ? S.tl.lanes.filter(l => l.kind === 'audio' && (l.id === 'A1' || l.clips > 0)) : [];
function laneTitle(l) {
  if (l.id === 'A1') return ['Main track', "The clips' own sound"];
  const names = [...new Set(S.tl.clips.filter(c => c.track === l.id).sort(byStart).map(c => c.name))];
  return [names[0] || l.id, `Lane ${l.id}` + (names.length > 1 ? `, and ${names.length - 1} more` : '')];
}
/* A dragged fader sends its latest value; each drag is one undo step. Playback goes on. */
let mxWant = null, mxBusy = false;
async function queueMixer(lane, values, merge) {
  if (mxWant && mxWant.merge === merge && mxWant.lane === lane) Object.assign(mxWant.values, values);
  else { if (mxWant) await sendMixer(); mxWant = { lane, values: { ...values }, merge }; }
  if (mxBusy) return;
  mxBusy = true;
  try { while (mxWant) await sendMixer(); } finally { mxBusy = false; }
}
async function sendMixer() {
  const w = mxWant;
  mxWant = null;
  if (!w) return;
  // not through run(): that pauses playback, and the mixer is for listening while it plays
  const tl = await serial(() => invoke('set_mixer', w).catch(e => { toast(e, true); return null; }));
  if (tl) apply(tl);
}
const mixerOf = lane => lane === 'Mix' ? S.tl.mix : S.tl.lanes.find(l => l.id === lane);
function mixerSlider(lane, key, label, min, max, step, neutral, unit) {
  const row = document.createElement('label');
  row.className = 'prop';
  row.innerHTML = `<span title="Double-click to reset">${label}</span><input type="range" min="${min}" max="${max}" step="${step}"><input type="number" step="${step}" title="${unit}">`;
  const [range, num] = row.querySelectorAll('input');
  range.id = `mx-${lane}-${key}`;
  let k = nextKey(`mx${key}`);
  const send = (v, merge) => { num.value = v; queueMixer(lane, { [key]: +v }, merge); };
  range.addEventListener('input', () => send(range.value, k));
  range.addEventListener('change', () => { k = nextKey(`mx${key}`); range.blur(); });
  num.addEventListener('change', () => { if (Number.isFinite(+num.value)) { const v = clamp(+num.value, min, max); range.value = v; send(v, null); } });
  row.firstChild.addEventListener('dblclick', e => { e.preventDefault(); range.value = neutral; send(neutral, null); });
  return row;
}
function renderMixer() {
  if ($('#panel-mixer').hidden) return;
  const box = $('#mixer');
  if (!has()) { box.innerHTML = '<p class="hint">Add a video first: its sound, and the sound lanes under it, show up here.</p>'; MX.sig = ''; return; }
  const strips = soundLanes().map(l => [l.id, ...laneTitle(l)]).concat([['Mix', 'Mix', 'Everything you hear, as it is exported']]);
  const sig = strips.map(s => s.join('|')).join('/');
  if (sig !== MX.sig) {
    MX.sig = sig;
    box.innerHTML = '';
    for (const [lane, title, sub] of strips) {
      const st = document.createElement('div');
      st.className = 'strip' + (lane === 'Mix' ? ' mix' : '');
      st.dataset.lane = lane;
      st.innerHTML = '<div class="strip-head"><div><b></b><small></small></div></div><canvas class="meter" width="240" height="14" aria-hidden="true"></canvas>';
      st.querySelector('b').textContent = title;
      st.querySelector('small').textContent = sub;
      st.querySelector('small').title = sub;
      if (lane !== 'Mix') {
        const head = st.querySelector('.strip-head');
        for (const [key, letter, name] of [['muted', 'M', 'Mute'], ['solo', 'S', 'Solo']]) {
          const b = document.createElement('button');
          b.className = 'btn ms';
          b.id = `mx-${lane}-${key}`;
          b.textContent = letter;
          b.title = `${name} ${lane === 'A1' ? "the main track's sound" : 'this lane'}`;
          b.setAttribute('aria-label', b.title);
          b.addEventListener('click', () => {
            const l = mixerOf(lane);
            queueMixer(lane, { [key]: key === 'muted' ? l.on : !l.solo }, null);
          });
          head.appendChild(b);
        }
      }
      st.appendChild(mixerSlider(lane, 'volumeDb', 'Volume', -60, 12, 0.5, 0, 'dB'));
      if (lane !== 'Mix') st.appendChild(mixerSlider(lane, 'pan', 'Pan', -100, 100, 1, 0, 'left −100 … right 100'));
      box.appendChild(st);
    }
  }
  for (const st of box.children) {
    const lane = st.dataset.lane, l = mixerOf(lane);
    if (!l) continue;
    for (const inp of st.querySelectorAll('input')) {
      if (inp === document.activeElement && inp.type === 'number') continue;
      const key = inp.closest('.prop').querySelector('input[type="range"]').id.endsWith('pan') ? 'pan' : 'volumeDb';
      inp.value = +(+l[key]).toFixed(1);
    }
    const m = st.querySelector('[id$="-muted"]'), so = st.querySelector('[id$="-solo"]');
    if (m) m.setAttribute('aria-pressed', String(!l.on));
    if (so) so.setAttribute('aria-pressed', String(!!l.solo));
    // a lane that can't be heard because others are soloed looks quieter
    const anySolo = soundLanes().some(x => x.solo);
    st.classList.toggle('silent', lane !== 'Mix' && (!l.on || (anySolo && !l.solo)));
  }
  drawMeters();
}
/** New peaks (`[{lane, left, right}]`) into the meters; they fall back at 24 dB a second. */
function takeLevels(levels) {
  const now = performance.now(), fall = (now - (MX.at || now)) / 1000 * 24;
  MX.at = now;
  const db = v => v > 0 ? 20 * Math.log10(v) : -Infinity;
  for (const [lane, v] of MX.level) MX.level.set(lane, { left: v.left - fall, right: v.right - fall, hold: v.hold, holdAt: v.holdAt });
  for (const l of levels) {
    const was = MX.level.get(l.lane) || { left: -Infinity, right: -Infinity, hold: -Infinity, holdAt: 0 };
    const left = Math.max(was.left, db(l.left)), right = Math.max(was.right, db(l.right));
    const top = Math.max(left, right);
    const hold = top >= was.hold || now - was.holdAt > 1500 ? top : was.hold;
    MX.level.set(l.lane, { left, right, hold, holdAt: hold === top ? now : was.holdAt });
  }
  drawMeters();
}
function drawMeters() {
  for (const cv of document.querySelectorAll('#mixer .meter')) {
    const g = cv.getContext('2d'), w = cv.width, h = cv.height;
    const v = MX.level.get(cv.parentElement.dataset.lane);
    const x = db => clamp((db - METER_LO) / (METER_HI - METER_LO), 0, 1) * w;
    g.fillStyle = '#18181c';
    g.fillRect(0, 0, w, h);
    // green up to −12 dB, amber to 0, red past it
    const bar = (db, y, bh) => {
      const end = x(db);
      for (const [from, to, col] of [[METER_LO, -12, '#4fc46a'], [-12, 0, '#f4c443'], [0, METER_HI, '#ff5a4f']]) {
        const a = x(from), b = Math.min(end, x(to));
        if (b > a) { g.fillStyle = col; g.fillRect(a, y, b - a, bh); }
      }
    };
    const bh = (h - 3) / 2;
    if (v) { bar(v.left, 1, bh); bar(v.right, 2 + bh, bh); }
    g.fillStyle = 'rgba(255,255,255,.18)';
    for (const db of [-36, -24, -12, -6, 0]) g.fillRect(Math.round(x(db)), 0, 1, h);
    if (v && v.hold > METER_LO) { g.fillStyle = v.hold > 0 ? '#ff5a4f' : '#e8e6ea'; g.fillRect(Math.round(x(v.hold)) - 1, 0, 2, h); }
  }
}
/** While playing with the Mixer open: the levels since the last look. */
let metersBusy = false;
async function pollMeters() {
  if (metersBusy || $('#panel-mixer').hidden) return;
  metersBusy = true;
  try { takeLevels(await invoke('meters')); } catch (_) { /* no levels this time */ } finally { metersBusy = false; }
}
/** Stopped: the meters fall to nothing. */
function settleMeters() {
  cancelAnimationFrame(MX.raf);
  const step = () => {
    if (S.playing) return;
    takeLevels([]);
    if ([...MX.level.values()].some(v => Math.max(v.left, v.right) > METER_LO)) MX.raf = requestAnimationFrame(step);
    else { MX.level.clear(); drawMeters(); }
  };
  MX.raf = requestAnimationFrame(step);
}

/* ---------- keyframes: an animated clip's properties as they are at the playhead ---------- */
const LIVE = { id: null, t: -1, props: null, seq: 0 };
const KF_EPS = () => frameStep() / 2;
/** The clip, with its animated properties as they are at the playhead (once props_at has said). */
function liveClip(c) {
  if (!c || LIVE.id !== c.id || !LIVE.props || Math.abs(LIVE.t - S.t) > 1e-6) return c;
  const p = {};
  for (const [k, v] of Object.entries(LIVE.props)) if (v != null) p[k] = v;
  return { ...c, props: { ...c.props, ...p }, live: LIVE.props };
}
const keyTimes = c => [...new Set((c.keys || []).flatMap(k => k.times))].sort((a, b) => a - b);
/** A property's keyframe state at the playhead: 'on' (a keyframe here), 'anim' (animated), ''. */
function kfState(c, prop) {
  const k = (c.keys || []).find(x => x.prop === prop);
  if (!k) return '';
  return k.times.some(t => Math.abs(t - S.t) < KF_EPS()) ? 'on' : 'anim';
}
let liveTimer = 0;
function refreshLive() {
  clearTimeout(liveTimer);
  const one = S.sel.size === 1 ? clipById([...S.sel][0]) : null;
  if (!one || isCap(one) || !(one.keys || []).length || S.playing) { LIVE.id = null; return; }
  liveTimer = setTimeout(async () => {
    const seq = ++LIVE.seq, t = S.t;
    try {
      const props = await invoke('props_at', { clip: one.id, seconds: t });
      if (seq !== LIVE.seq) return;
      Object.assign(LIVE, { id: one.id, t, props });
      renderProps();
      drawOverlay();
    } catch (_) { /* the clip went */ }
  }, 30);
}
/** The diamond beside a property: adds a keyframe at the playhead, or removes the one there. */
function kfButton(ids, prop) {
  const b = document.createElement('button');
  b.className = 'kf';
  b.innerHTML = '<svg viewBox="0 0 12 12" aria-hidden="true"><path d="M6 1l5 5-5 5-5-5z"/></svg>';
  b.addEventListener('click', e => {
    e.preventDefault();
    e.stopPropagation();
    edit('toggle_keyframe', { clip: ids[0], prop, seconds: S.t });
  });
  controls.push({
    el: b,
    read: c => kfState(c, prop),
    write: (el, v) => {
      el.dataset.state = v;
      el.title = v === 'on' ? 'Remove the keyframe here' : v === 'anim' ? 'Add a keyframe here (or change the value: it makes one)' : 'Animate it: a keyframe here, with the value now';
      el.setAttribute('aria-label', el.title);
    },
  });
  return b;
}
/** The keyframe singled out on the timeline goes (every property keyed there). */
async function deleteKeyframe() {
  const k = S.kf;
  if (!k) return;
  if (await edit('delete_keyframes', { clip: k.clip, seconds: k.t })) { S.kf = null; renderSelection(); }
}
/** Previous / next keyframe, and whether the clip's keyframes ease in and out. */
function keyframeControls(s, c) {
  const times = keyTimes(c);
  if (S.kf && S.kf.clip === c.id) {
    const names = { position: 'Position', scale: 'Scale', rotation: 'Rotation', opacity: 'Opacity', volume: 'Volume' };
    const here = (c.keys || []).filter(k => k.times.some(t => Math.abs(t - S.kf.t) < KF_EPS())).map(k => names[k.prop] || k.prop);
    const p = document.createElement('p');
    p.className = 'kf-sel';
    p.textContent = `Selected keyframe at ${fmtFrames(S.kf.t)}: ${here.join(', ') || 'none'}. Drag its diamond to move it.`;
    s.appendChild(p);
    s.appendChild(buttons([['Delete keyframe', 'Remove this keyframe (Delete)', deleteKeyframe]]));
  }
  if (!times.length) {
    const p = document.createElement('p');
    p.className = 'hint';
    p.textContent = 'Press ◇ beside a property to animate it: it keeps its value here, and changing it at another time makes the next keyframe.';
    s.appendChild(p);
    return;
  }
  const go = dir => {
    const t = dir < 0 ? [...keyTimes(clipById(c.id))].reverse().find(x => x < S.t - KF_EPS()) : keyTimes(clipById(c.id)).find(x => x > S.t + KF_EPS());
    if (t == null) { toast(dir < 0 ? 'No keyframe before the playhead' : 'No keyframe after the playhead'); return; }
    seek(t);
    scrollToTime(t);
  };
  const row = buttons([
    ['◀ Previous', 'Go to the previous keyframe', () => go(-1)],
    ['Next ▶', 'Go to the next keyframe', () => go(1)],
    ['Smooth', 'Ease in and out of every keyframe, rather than changing at a steady rate', () => {
      const cc = clipById(c.id);
      edit('set_easing', { clip: c.id, smooth: !(cc.keys || []).every(k => k.smooth) });
    }],
  ]);
  const smooth = row.lastChild;
  controls.push({ el: smooth, read: x => (x.keys || []).length > 0 && x.keys.every(k => k.smooth), write: (el, v) => el.setAttribute('aria-pressed', String(!!v)) });
  const hold = document.createElement('button');
  hold.id = 'kf-hold';
  hold.textContent = 'Hold the last value';
  hold.title = 'Keep the values of the last keyframe to the end of the clip, rather than going straight back to the first keyframe’s';
  hold.onclick = () => edit('set_hold', { clip: c.id, hold: !clipById(c.id).hold });
  row.appendChild(hold);
  controls.push({ el: hold, read: x => !!x.hold, write: (el, v) => el.setAttribute('aria-pressed', String(!!v)) });
  s.appendChild(row);
  const p = document.createElement('p');
  p.className = 'hint';
  p.id = 'kf-after';
  controls.push({ el: p, read: x => !!x.hold, write: (el, v) => { el.textContent = v ? 'After the last keyframe the clip keeps its last values.' : 'After the last keyframe the clip goes straight back to the values at its first keyframe.'; } });
  s.appendChild(p);
}

/* ---------- timeline layout: lanes above and below the main track ---------- */
const minPx = () => {
  const d = dur(), w = scroller().clientWidth - 2 * PAD - 40;
  return Math.min(d > 0 ? Math.max(0.02, w / d) : 60, MAX_PX / 2);
};
function fitZoom(relayout = true) {
  const d = dur(), w = scroller().clientWidth - 2 * PAD - 40;
  S.px = d > 0 ? Math.min(MAX_PX, Math.max(0.02, w / d)) : 60;
  scroller().scrollLeft = 0;
  if (relayout) layoutTimeline();
}
/** Zooms to `px` keeping the time under `clientX` (or the playhead, or the middle) in place. */
function setPx(px, clientX) {
  const sc = scroller(), r = sc.getBoundingClientRect();
  const lo = Math.min(minPx(), S.px);
  px = Math.min(MAX_PX, Math.max(lo, px));
  let ax;
  if (clientX != null) ax = clientX - r.left;
  else {
    const xp = PAD + S.t * S.px - sc.scrollLeft;
    ax = xp >= 0 && xp <= sc.clientWidth ? xp : sc.clientWidth / 2;
  }
  const at = (sc.scrollLeft + ax - PAD) / S.px;
  S.px = px;
  layoutTimeline();
  sc.scrollLeft = PAD + at * S.px - ax;
  drawVisible();
}
function syncZoomSlider() {
  const lo = minPx();
  const z = S.px <= lo ? 0 : Math.log(S.px / lo) / Math.log(MAX_PX / lo);
  $('#zoom').value = Math.round(Math.min(1, Math.max(0, z)) * 100);
}
function zoomFromSlider() {
  const lo = minPx();
  setPx(lo * Math.pow(MAX_PX / lo, +$('#zoom').value / 100));
}

/** The lanes on screen: overlay lanes with clips (highest on top), the main track, sound lanes. */
function layoutRows() {
  const used = new Set(visibleClips().map(c => c.track));
  const lanes = S.tl ? S.tl.lanes : [];
  const vids = lanes.filter(l => l.kind === 'video' && l.id !== 'V1' && used.has(l.id)).sort((a, b) => laneNo(b.id) - laneNo(a.id));
  const auds = lanes.filter(l => l.kind === 'audio' && l.id !== 'A1' && used.has(l.id)).sort((a, b) => laneNo(a.id) - laneNo(b.id));
  const rows = [];
  let y = ROW.top;
  if (S.tl && S.tl.captions.exists && S.tl.captions.items.length) { rows.push({ id: 'C1', kind: 'captions', top: y, h: ROW.caption }); y += ROW.caption + ROW.gap * 2; }
  for (const l of vids) { rows.push({ id: l.id, kind: 'video', top: y, h: ROW.video }); y += ROW.video + ROW.gap; }
  if (vids.length) y += ROW.gap;
  rows.push({ id: 'V1', kind: 'main', top: y, h: ROW.main });
  y += ROW.main + ROW.gap;
  if (auds.length) y += ROW.gap;
  for (const l of auds) { rows.push({ id: l.id, kind: 'audio', top: y, h: ROW.audio }); y += ROW.audio + ROW.gap; }
  S.rows = rows;
  return y;
}
const rowOf = id => S.rows.find(r => r.id === id);
const mainRow = () => rowOf('V1');

function renderTrack() {
  const height = layoutRows();
  const rowsEl = $('#rows'), heads = $('#heads-inner');
  rowsEl.innerHTML = '';
  heads.innerHTML = '';
  rowsEl.style.height = height + 'px';
  heads.style.height = (height + 26) + 'px';
  S.els.clear();
  for (const r of S.rows) {
    const row = document.createElement('div');
    row.className = 'row ' + (r.kind === 'main' ? 'track main-track' : 'lane');
    if (r.kind === 'main') row.id = 'track';
    row.dataset.lane = r.id;
    row.style.top = r.top + 'px';
    row.style.height = r.h + 'px';
    rowsEl.appendChild(row);
    heads.appendChild(laneHead(r));
  }
  if (!mainClips().length) {
    $('#track').innerHTML = '<div class="track-empty">Drop videos here, or press + on a Media card</div>';
  }
  for (const c of laneThings().sort(byStart)) {
    const row = rowsEl.querySelector(`[data-lane="${c.track}"]`);
    if (!row) continue;
    const el = makeClipEl(c);
    row.appendChild(el);
    S.els.set(c.id, el);
  }
  // each cut: a square to give it a transition, and the span of the one it has
  const track = $('#track');
  for (const c of mainCuts()) {
    const t = transitionAt(c.from);
    if (t) {
      const sp = document.createElement('div');
      sp.className = 'tr-span';
      sp.dataset.from = c.from;
      track.appendChild(sp);
    }
    const b = document.createElement('button');
    b.className = 'cut' + (t ? ' has' : '');
    b.dataset.from = c.from;
    b.innerHTML = t ? ICON.transition : ICON.plus;
    b.title = t ? `${t.name}, ${fmtLen(t.end - t.start)}: click to change it` : 'Add a transition here';
    b.setAttribute('aria-label', b.title);
    b.addEventListener('pointerdown', e => {
      if (e.button !== 0) return;
      e.stopPropagation();
      e.preventDefault();
      selectCut(c.from);
      showTab('transitions');
    });
    track.appendChild(b);
  }
  $('#insert-mark').style.top = (26 + mainRow().top) + 'px';
  $('#insert-mark').style.height = ROW.main + 'px';
}
/** The lane's switch on the left: hide or show a picture lane, mute or unmute a sound lane. */
function laneHead(r) {
  const head = document.createElement('div');
  head.className = 'head';
  head.style.top = (26 + r.top) + 'px';
  head.style.height = r.h + 'px';
  if (r.kind === 'captions') {
    const on = S.tl.captions.on;
    const b = document.createElement('button');
    b.className = 'tool' + (on ? '' : ' off');
    b.innerHTML = on ? ICON.eye : ICON.eyeOff;
    b.title = on ? 'Hide the captions (and leave them out of exports)' : 'Show the captions';
    b.setAttribute('aria-label', b.title);
    b.addEventListener('click', () => edit('set_captions_on', { on: !on }));
    head.appendChild(b);
    return head;
  }
  const lane = r.kind === 'main' ? 'A1' : r.id;
  const sound = r.kind !== 'video';
  const on = laneOn(lane);
  const b = document.createElement('button');
  b.className = 'tool' + (on ? '' : ' off');
  b.innerHTML = sound ? (on ? ICON.sound : ICON.muted) : (on ? ICON.eye : ICON.eyeOff);
  const what = r.kind === 'main' ? "the main track's sound" : (sound ? 'this sound lane' : 'this lane');
  b.title = sound ? `${on ? 'Mute' : 'Unmute'} ${what}` : `${on ? 'Hide' : 'Show'} ${what}`;
  b.setAttribute('aria-label', b.title);
  b.addEventListener('click', () => edit('set_lane', { lane, on: !on }));
  head.appendChild(b);
  return head;
}
function makeClipEl(c) {
  const el = document.createElement('div');
  const kind = isCap(c) ? 'caption' : isMain(c) ? 'main' : c.kind === 'audio' ? 'audio' : c.kind === 'text' ? 'text' : 'overlay';
  el.className = 'clip ' + kind;
  el.dataset.id = c.id;
  const handles = '<div class="handle in" title="Drag to trim the start"></div><div class="handle out" title="Drag to trim the end"></div>';
  if (kind === 'main') el.innerHTML = '<div class="pics"><canvas></canvas></div><div class="snd"><canvas></canvas><div class="fade in"></div><div class="fade out"></div></div><span class="label"></span>' + handles;
  else if (kind === 'overlay') el.innerHTML = '<div class="pics"><canvas></canvas></div><span class="label"></span>' + handles;
  else if (kind === 'audio') el.innerHTML = '<div class="wave"><canvas></canvas></div><div class="fade in"></div><div class="fade out"></div><span class="label"></span>' + handles;
  else el.innerHTML = '<span class="label"></span>' + handles;
  const name = kind === 'caption' ? c.text.replace(/\n/g, ' ') : kind === 'text' ? (c.props.text && c.props.text.text.split('\n')[0]) || 'Text' : c.name;
  el.querySelector('.label').textContent = kind === 'caption' ? name : `${c.graphic ? TOOL_NAMES[c.graphic] || 'Graphic' : name} · ${fmtLen(c.end - c.start)}`;
  if (c.graphic) { const t = document.createElement('span'); t.className = 'gr-tag'; t.textContent = 'Graphic'; el.appendChild(t); el.title = `${name}: made in Graphics. Select it and press Edit in Graphics to change it.`; }
  if (kind === 'caption') el.title = c.text;
  if (kind === 'main') el.querySelector('.snd').classList.toggle('none', !hasSound(c));
  if (kind === 'audio' || kind === 'main') {
    // a main clip's fades are its sound's
    const p = (soundOf(c) || c).props;
    el.querySelector('.fade.in').style.width = (p.fadeIn * S.px) + 'px';
    el.querySelector('.fade.out').style.width = (p.fadeOut * S.px) + 'px';
  }
  el.classList.toggle('muted', kind === 'caption' ? !S.tl.captions.on : !laneOn(c.track));
  if (kind === 'main') el.querySelector('.snd').classList.toggle('off', !laneOn('A1'));
  const times = keyTimes(c);
  if (times.length && c.end > c.start) {
    const kfs = document.createElement('div');
    kfs.className = 'kfs';
    for (const t of times) {
      const d = document.createElement('i');
      d.style.left = ((t - c.start) / (c.end - c.start) * 100) + '%';
      d.dataset.t = t;
      d.title = `Keyframe at ${fmtFrames(t)}: click to select it (Delete removes it), drag to move it`;
      d.classList.toggle('sel', isSelKf(c.id, t));
      d.addEventListener('pointerdown', e => kfPointerDown(e, c, t, d));
      kfs.appendChild(d);
    }
    el.appendChild(kfs);
  }
  if (c.beats && c.beats.length && c.end > c.start) {
    const row = document.createElement('div');
    row.className = 'beats';
    for (const t of c.beats) {
      const d = document.createElement('i');
      d.style.left = ((t - c.start) / (c.end - c.start) * 100) + '%';
      row.appendChild(d);
    }
    el.appendChild(row);
  }
  el.addEventListener('pointerdown', e => clipPointerDown(c, el, e));
  return el;
}
let extraWidth = 0;   // room a drag past the end needs
function layoutTimeline() {
  const sc = scroller();
  const end = Math.max(dur(), ...laneThings().map(c => c.end));
  const width = Math.max(sc.clientWidth, PAD + end * S.px + 240 + extraWidth);
  content().style.width = width + 'px';
  layoutClips();
  syncZoomSlider();
  updateTime();
  drawVisible();
}
function layoutClips() {
  for (const c of laneThings()) {
    const el = S.els.get(c.id);
    if (!el) continue;
    const g = geom(c);
    const w = Math.max(2, (g.end - g.start) * S.px);
    el.style.left = (g.start * S.px) + 'px';
    el.style.width = w + 'px';
    el.style.transform = g.dy ? `translateY(${g.dy}px)` : '';
    el.classList.toggle('narrow', w < 48);
  }
  const track = $('#track');
  if (!track) return;
  // a drag moves the clips: the cuts come back when it lands
  track.classList.toggle('dragging-now', S.geom.size > 0);
  for (const el of track.querySelectorAll('.cut, .tr-span')) {
    const from = +el.dataset.from, t = transitionAt(from), cut = mainCuts().find(c => c.from === from);
    if (!cut) continue;
    if (el.classList.contains('cut')) el.style.left = (cut.at * S.px) + 'px';
    else if (t) { el.style.left = (t.start * S.px) + 'px'; el.style.width = Math.max(2, (t.end - t.start) * S.px) + 'px'; }
  }
}
let drawQueued = 0;
/** Ruler, filmstrips and waveforms for what is on screen, once per frame at most. */
function drawVisible() {
  if (drawQueued) return;
  drawQueued = requestAnimationFrame(() => { drawQueued = 0; drawRuler(); drawStrips(); });
}

/* ---------- ruler (a canvas the width of the view, redrawn as it scrolls) ---------- */
let rulerColors = null;
function drawRuler() {
  const sc = scroller(), r = $('#ruler');
  let c = r.querySelector('canvas');
  if (!c) { c = document.createElement('canvas'); r.appendChild(c); }
  const dpr = window.devicePixelRatio || 1, w = sc.clientWidth, h = 26;
  c.style.left = sc.scrollLeft + 'px';
  c.style.width = w + 'px';
  if (c.width !== Math.round(w * dpr) || c.height !== Math.round(h * dpr)) { c.width = Math.round(w * dpr); c.height = Math.round(h * dpr); }
  const g = c.getContext('2d');
  g.setTransform(dpr, 0, 0, dpr, 0, 0);
  g.clearRect(0, 0, w, h);
  if (!has()) return;
  if (!rulerColors) {
    const cs = getComputedStyle(document.documentElement);
    rulerColors = { line: cs.getPropertyValue('--line').trim(), faint: cs.getPropertyValue('--faint').trim(), mono: cs.getPropertyValue('--mono').trim() };
  }
  // a labelled tick whichever step keeps labels about 90 px apart; frames when zoomed right in
  const f = frameStep();
  const steps = [f, 2 * f, 5 * f, 10 * f, 0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 1200, 1800, 3600];
  const major = steps.find(s => s * S.px >= 90) || 3600;
  const minor = major / 5;
  const t0 = Math.max(0, (sc.scrollLeft - PAD) / S.px), t1 = (sc.scrollLeft + w - PAD) / S.px;
  g.font = `10.5px ${rulerColors.mono}`;
  g.textBaseline = 'top';
  // count ticks rather than adding up `minor`, which drifts
  for (let i = Math.floor(t0 / minor); i * minor <= t1 + minor; i++) {
    const t = i * minor, isMajor = i % 5 === 0;
    const x = Math.round(PAD + t * S.px - sc.scrollLeft) + 0.5;
    g.fillStyle = isMajor ? rulerColors.faint : rulerColors.line;
    g.fillRect(x - 0.5, h - (isMajor ? 11 : 6), 1, isMajor ? 11 : 6);
    if (isMajor) {
      g.fillStyle = rulerColors.faint;
      const lab = major < 1 && Math.abs(t - Math.round(t)) > 1e-6 ? `${Math.round((t % 1) / f)}f` : fmt(t).replace(/^00:/, '');
      g.fillText(lab, x + 4, 4);
    }
  }
}

/* ---------- filmstrips (media pictures, cached by media and time) and waveforms ---------- */
const Thumbs = {
  cache: new Map(),       // "item:time" → canvas, or null when it couldn't be read
  queue: [],
  running: false,
  clear() { this.cache.clear(); this.queue = []; mediaThumbs.clear(); mediaSig = ''; },
};
const Peaks = {
  cache: new Map(),       // item → Uint8Array (1/100 s each), or null when there is none
  pending: new Set(),
  clear() { this.cache.clear(); this.pending.clear(); },
  get(item) {
    if (this.cache.has(item)) return this.cache.get(item);
    if (!this.pending.has(item)) {
      this.pending.add(item);
      invoke('peaks', { item })
        .then(buf => { this.cache.set(item, new Uint8Array(buf)); })
        .catch(() => { this.cache.set(item, null); })
        .finally(() => { this.pending.delete(item); drawStripsSoon(); });
    }
    return undefined;
  },
};
const view = () => { const sc = scroller(); return [sc.scrollLeft - PAD - 200, sc.scrollLeft - PAD + sc.clientWidth + 200]; };
function drawStrips() {
  const [viewL, viewR] = view();
  const need = [];
  for (const c of visibleClips()) {
    const el = S.els.get(c.id);
    if (!el) continue;
    if (el.querySelector('.pics')) drawStrip(c, el, viewL, viewR, need);
    const wave = el.querySelector('.wave canvas, .snd canvas');
    if (wave) drawWave(isMain(c) ? soundOf(c) : c, c, wave, viewL, viewR);
  }
  Thumbs.queue = [...new Set(need)];
  pumpThumbs();
}
/** Fits a lane canvas to the part of clip `g` that is on screen; returns [left, right] or null. */
function fitPart(cv, g, viewL, viewR, h) {
  const L = g.start * S.px, R = g.end * S.px;
  const a = Math.max(L, viewL), b = Math.min(R, viewR);
  if (b <= a || h <= 0) { cv.style.display = 'none'; return null; }
  const dpr = window.devicePixelRatio || 1;
  cv.style.display = '';
  cv.style.left = (a - L) + 'px';
  cv.style.width = (b - a) + 'px';
  const pw = Math.round((b - a) * dpr), ph = Math.round(h * dpr);
  if (cv.width !== pw || cv.height !== ph) { cv.width = pw; cv.height = ph; }
  const ctx = cv.getContext('2d');
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  return [a, b, L];
}
function drawStrip(c, el, viewL, viewR, need) {
  const g = geom(c);
  const pics = el.querySelector('.pics'), cv = pics.querySelector('canvas');
  const h = pics.clientHeight;
  const part = fitPart(cv, g, viewL, viewR, h);
  if (!part) return;
  const [a, b, L] = part, ctx = cv.getContext('2d');
  ctx.fillStyle = '#0b0b0d';
  ctx.fillRect(0, 0, b - a, h);
  const m = mediaOf(c.item);
  const aspect = m && m.width && m.height ? m.width / m.height : S.tl.width / Math.max(1, S.tl.height);
  const tileW = Math.max(16, h * aspect);
  // a picture per tile, its time quantized to a step about one tile long
  const rate = (g.sourceOut - g.sourceIn) / Math.max(1e-6, g.end - g.start);
  const tileDur = tileW / S.px * rate;
  const q = [0.04, 0.1, 0.2, 0.5, 1, 2, 5, 10, 20, 30, 60, 120, 300, 600].find(s => s >= tileDur) || 600;
  const last = Math.max(0, (m ? m.duration : g.sourceOut) - frameStep());
  for (let k = Math.floor((a - L) / tileW); k * tileW < b - L; k++) {
    const src = Math.min(last, g.sourceIn + (k + 0.5) * tileDur);
    const key = `${c.item}:${+(Math.round(src / q) * q).toFixed(3)}`;
    const x = L + k * tileW - a;
    const img = Thumbs.cache.get(key);
    if (img) {
      drawCover(ctx, img, x, 0, tileW, h);
      ctx.fillStyle = 'rgba(0,0,0,.35)';
      ctx.fillRect(x + tileW - 1, 0, 1, h);
    } else if (img === undefined) need.push(key);
  }
}
/** The sound clip `snd`'s waveform, drawn in clip `c`'s element (a main clip draws its sound's). */
function drawWave(snd, c, cv, viewL, viewR) {
  const g = geom(c);
  const h = cv.parentElement.clientHeight;
  const part = fitPart(cv, g, viewL, viewR, h);
  if (!part || !snd) return;
  const [a, b, L] = part, ctx = cv.getContext('2d');
  ctx.clearRect(0, 0, b - a, h);
  const peaks = Peaks.get(snd.item);
  if (!peaks) return;
  // louder or quieter with the clip's volume, as it will sound
  const gain = Math.pow(10, (snd.props.volumeDb || 0) / 20);
  const rate = (g.sourceOut - g.sourceIn) / Math.max(1e-6, g.end - g.start);
  ctx.fillStyle = getComputedStyle(document.documentElement).getPropertyValue('--sound-wave').trim() || '#46c99a';
  const mid = h / 2;
  for (let x = 0; x < b - a; x++) {
    const t0 = g.sourceIn + (a - L + x) / S.px * rate, t1 = g.sourceIn + (a - L + x + 1) / S.px * rate;
    let v = 0;
    for (let i = Math.floor(t0 * PEAKS_PER_SECOND), n = Math.max(i + 1, Math.ceil(t1 * PEAKS_PER_SECOND)); i < n && i < peaks.length; i++) v = Math.max(v, peaks[i]);
    // a gentle curve so quiet speech still shows
    const amp = Math.min(1, Math.sqrt(v / 255 * gain)) * (h / 2 - 1);
    if (amp > 0.3) ctx.fillRect(x, mid - amp, 1, amp * 2);
  }
}
async function pumpThumbs() {
  if (Thumbs.running) return;
  Thumbs.running = true;
  try {
    while (Thumbs.queue.length) {
      const key = Thumbs.queue.shift();
      if (Thumbs.cache.has(key)) continue;
      const [item, t] = key.split(':').map(Number);
      try {
        Thumbs.cache.set(key, toCanvas(toImage(await invoke('thumb', { item, seconds: t, maxWidth: THUMB_W }))));
      } catch (e) {
        Thumbs.cache.set(key, null);
      }
      if (Thumbs.cache.size > 900) {
        let n = 150;
        for (const k of Thumbs.cache.keys()) { Thumbs.cache.delete(k); if (--n <= 0) break; }
      }
      drawStripsSoon();
      // while playing, the preview comes first
      if (S.playing) await sleep(150);
    }
  } finally {
    Thumbs.running = false;
  }
}
let stripsQueued = 0;
function drawStripsSoon() {
  if (stripsQueued) return;
  stripsQueued = setTimeout(() => {
    stripsQueued = 0;
    const [viewL, viewR] = view();
    for (const c of visibleClips()) {
      const el = S.els.get(c.id);
      if (!el) continue;
      if (el.querySelector('.pics')) drawStrip(c, el, viewL, viewR, []);
      const wave = el.querySelector('.wave canvas, .snd canvas');
      if (wave) drawWave(isMain(c) ? soundOf(c) : c, c, wave, viewL, viewR);
    }
  }, 60);
}

/* ---------- time, playhead and tools ---------- */
function updateTime() {
  $('#tc-now').textContent = fmtFrames(S.t);
  $('#playhead').style.left = (PAD + S.t * S.px) + 'px';
  renderTools();
  markWord();
  if (!S.playing && LIVE.t !== S.t) { refreshLive(); syncKfButtons(); }
  if (!S.playing && !$('#panel-filters').hidden) filtersSoon();
}
/** The diamonds follow the playhead before props_at answers. */
function syncKfButtons() {
  const one = S.sel.size === 1 ? clipById([...S.sel][0]) : null;
  if (!one) return;
  for (const k of controls) if (k.el.classList && k.el.classList.contains('kf') && k.write) k.write(k.el, k.read(one));
}
/** The clips a split at the playhead would cut: the selected ones, or the main track's. */
function cutTargets() {
  const half = frameStep() / 2;
  const pool = S.sel.size ? visibleClips().filter(c => S.sel.has(c.id)) : mainClips();
  return pool.filter(c => c.start + half < S.t && c.end - half > S.t);
}
let lastAt = -1;
function renderTools() {
  if (!S.tl) return;
  const cut = has() && cutTargets().length > 0;
  for (const id of ['#split-btn', '#delleft-btn', '#delright-btn']) $(id).disabled = !cut;
  $('#delete-btn').disabled = S.sel.size === 0 && !transitionAt(S.cut);
  $('#undo-btn').disabled = !S.tl.canUndo;
  $('#redo-btn').disabled = !S.tl.canRedo;
  $('#undo-btn').title = S.tl.undoLabel ? `Undo ${S.tl.undoLabel} (Ctrl+Z)` : 'Undo (Ctrl+Z)';
  $('#redo-btn').title = S.tl.redoLabel ? `Redo ${S.tl.redoLabel} (Ctrl+Shift+Z)` : 'Redo (Ctrl+Shift+Z)';
  $('#snap-btn').setAttribute('aria-pressed', String(S.snap));
  // the player's frame follows whether the selected clip is under the playhead
  const at = stageClip() ? 1 : 0;
  if (at !== lastAt) { lastAt = at; drawOverlay(); }
}
function setPlayIcon() {
  $('#play-icon').innerHTML = S.playing ? '<path d="M7 5h4v14H7zM13 5h4v14h-4z"/>' : '<path d="M8 5v14l11-7z"/>';
  $('#play-btn').setAttribute('aria-label', S.playing ? 'Pause' : 'Play');
}
function scrollToTime(t) {
  const sc = scroller(), x = PAD + t * S.px;
  if (x < sc.scrollLeft + 40 || x > sc.scrollLeft + sc.clientWidth - 40) sc.scrollLeft = Math.max(0, x - sc.clientWidth / 2);
}

/* ---------- selection, properties and details ---------- */
/** `kf`: a keyframe singled out on the timeline ({clip, t}), else null. */
function select(ids, kf = null) {
  S.sel = new Set(ids);
  S.cut = null;
  S.kf = kf;
  renderSelection();
}
const isSelKf = (clip, t) => !!S.kf && S.kf.clip === clip && Math.abs(S.kf.t - t) < KF_EPS();
/** A keyframe's diamond: a click singles it out (its clip selected, the playhead on it); a drag
 *  moves it along the clip, frame by frame. */
function kfPointerDown(e, c, t, d) {
  if (e.button !== 0) return;
  e.stopPropagation();
  e.preventDefault();
  select([c.id], { clip: c.id, t });
  seek(t);
  const x0 = e.clientX, step = frameStep();
  const lo = c.start, hi = Math.max(c.start, c.end - step);
  let to = t, moved = false;
  const at = ev => Math.min(hi, Math.max(lo, Math.round((t + (ev.clientX - x0) / S.px) / step) * step));
  const move = ev => {
    if (!moved && Math.abs(ev.clientX - x0) < 3) return;
    moved = true;
    S.dragging = true;
    to = at(ev);
    d.style.left = ((to - c.start) / (c.end - c.start) * 100) + '%';
    d.title = `Move to ${fmtFrames(to)}`;
  };
  const up = () => {
    window.removeEventListener('pointermove', move);
    window.removeEventListener('pointerup', up);
    S.dragging = false;
    if (!moved || Math.abs(to - t) < KF_EPS()) { d.style.left = ((t - c.start) / (c.end - c.start) * 100) + '%'; return; }
    edit('move_keyframes', { clip: c.id, from: t, to }).then(tl => { if (tl) { S.kf = { clip: c.id, t: to }; seek(to); renderSelection(); } });
  };
  window.addEventListener('pointermove', move);
  window.addEventListener('pointerup', up);
}
function renderSelection() {
  refreshLive();
  for (const [id, el] of S.els) {
    el.classList.toggle('sel', S.sel.has(id));
    for (const d of el.querySelectorAll('.kfs i')) d.classList.toggle('sel', isSelKf(id, +d.dataset.t));
  }
  for (const el of document.querySelectorAll('#track .cut, #track .tr-span')) el.classList.toggle('sel', +el.dataset.from === S.cut);
  renderTransitionsPanel();
  renderTools();
  renderProps();
  renderDetails();
  drawOverlay();
}
function selectedClips() { return laneThings().filter(c => S.sel.has(c.id)).sort(byStart); }

/* Property changes: coalesced so a dragged slider sends its latest value, and merged into one undo
   step per drag (the key changes when the drag ends). */
const propQueue = [];
let propBusy = false, keySeq = 0;
const nextKey = name => `${name}:${++keySeq}`;
function queueProps(ids, props, merge = null) {
  // an animated property gets its keyframe at the playhead (kept on the clip)
  const c = ids.length === 1 ? clipById(ids[0]) : null;
  if (c && props.at == null) props = { ...props, at: clamp(S.t, c.start, Math.max(c.start, c.end - frameStep())) };
  const last = propQueue[propQueue.length - 1];
  if (last && merge && last.merge === merge && String(last.ids) === String(ids)) Object.assign(last.props, props);
  else propQueue.push({ ids, props: { ...props }, merge });
  if (!propBusy) flushProps();
}
async function flushProps() {
  propBusy = true;
  try {
    while (propQueue.length) {
      const { ids, props, merge } = propQueue.shift();
      await edit('set_props', { clips: ids, props, merge });
    }
  } finally {
    propBusy = false;
  }
}

/* ---------- titles: the engine's templates and the user's own, in the Text tab ---------- */
const TI = { list: null, thumbs: new Map(), controls: null, clip: null, sig: '' };
async function renderTitles(reload = false) {
  const box = $('#titles');
  if (reload || !TI.list) {
    try { TI.list = await invoke('titles'); } catch (e) { box.innerHTML = '<p class="hint">The titles could not be loaded.</p>'; return; }
  }
  box.innerHTML = '';
  const cats = [...new Set(TI.list.map(t => t.category))];
  for (const cat of cats) {
    const h = document.createElement('h3');
    h.className = 'sub';
    h.textContent = cat;
    box.appendChild(h);
    const grid = document.createElement('div');
    grid.className = 'title-grid';
    for (const t of TI.list.filter(x => x.category === cat)) {
      const b = document.createElement('button');
      b.className = 'title-tile';
      b.id = `title-${t.id.replace(/[^a-z0-9-]/gi, '-')}`;
      b.title = `${t.description || t.name} Click to add it at the playhead.`;
      b.innerHTML = '<span class="pic"></span><span class="name"></span>';
      b.querySelector('.name').textContent = t.name;
      b.addEventListener('click', () => addTitle(t));
      if (t.mine) {
        const x = document.createElement('span');
        x.className = 'remove';
        x.textContent = '×';
        x.title = `Remove ${t.name} from your titles`;
        x.addEventListener('click', async e => {
          e.stopPropagation();
          try { await invoke('remove_title', { id: t.id }); } catch (err) { toast(err, true); return; }
          TI.thumbs.delete(t.id);
          renderTitles(true);
        });
        b.appendChild(x);
      }
      grid.appendChild(b);
      paintTitle(t, b.querySelector('.pic'));
    }
    box.appendChild(grid);
  }
}
async function paintTitle(t, el) {
  let src = TI.thumbs.get(t.id);
  if (!src) {
    try { src = 'data:image/png;base64,' + await invoke('title_thumbnail', { id: t.id, width: 240 }); } catch (_) { return; }
    TI.thumbs.set(t.id, src);
  }
  const img = new Image();
  img.onload = () => {
    // the drawn part of the frame, with a margin, so a lower third isn't a speck in a corner
    const w = img.naturalWidth, h = img.naturalHeight, c = document.createElement('canvas');
    c.width = w; c.height = h;
    const g = c.getContext('2d');
    g.drawImage(img, 0, 0);
    const px = g.getImageData(0, 0, w, h).data;
    let x0 = w, y0 = h, x1 = -1, y1 = -1;
    for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) if (px[(y * w + x) * 4 + 3] > 8) { x0 = Math.min(x0, x); x1 = Math.max(x1, x); y0 = Math.min(y0, y); y1 = Math.max(y1, y); }
    if (x1 < 0) { el.appendChild(img); return; }
    const m = Math.max(4, (x1 - x0) * 0.06), bw = Math.min(w, x1 - x0 + 2 * m), bh = Math.min(h, y1 - y0 + 2 * m);
    // keep the tile's 16:9
    const cw = Math.max(bw, bh * 16 / 9), ch = cw * 9 / 16;
    const cx = clamp((x0 + x1) / 2 - cw / 2, 0, Math.max(0, w - cw)), cy = clamp((y0 + y1) / 2 - ch / 2, 0, Math.max(0, h - ch));
    const out = document.createElement('canvas');
    out.width = 240; out.height = 135;
    out.getContext('2d').drawImage(c, cx, cy, Math.min(cw, w), Math.min(ch, h), 0, 0, 240, 135);
    el.appendChild(out);
  };
  img.src = src;
}
async function addTitle(t) {
  if (!has()) { toast('Add a video first: titles go over it.'); return; }
  await make('add_title', () => ({ id: t.id, seconds: S.t }));
}
/** A title's own controls (its words, colours, what shows), in Properties. */
async function titleControls(s, c) {
  const box = document.createElement('div');
  box.className = 'title-controls';
  s.appendChild(box);
  let list;
  try { list = await invoke('title_controls', { clip: c.id }); } catch (e) { box.innerHTML = '<p class="hint">This title has nothing to change.</p>'; return; }
  if (!list.length) { box.innerHTML = '<p class="hint">This title has nothing to change.</p>'; return; }
  const send = (id, value, merge = null) => edit('set_title', { clip: c.id, values: { [id]: value }, merge });
  for (const k of list) {
    const row = document.createElement('label');
    row.className = 'prop wide';
    row.innerHTML = '<span></span>';
    row.firstChild.textContent = k.name;
    let el;
    if (k.kind === 'text') {
      el = document.createElement('textarea');
      el.rows = 1;
      el.value = k.value || '';
      el.addEventListener('keydown', e => e.stopPropagation());
      el.addEventListener('change', () => send(k.id, el.value));
    } else if (k.kind === 'color') {
      el = document.createElement('input');
      el.type = 'color';
      el.value = k.value || '#ffffff';
      let key = nextKey('tcol');
      el.addEventListener('input', () => send(k.id, el.value, key));
      el.addEventListener('change', () => { key = nextKey('tcol'); });
    } else if (k.kind === 'checkbox') {
      row.className = 'check';
      row.innerHTML = '<input type="checkbox"> <span></span>';
      row.lastChild.textContent = k.name;
      el = row.firstChild;
      el.checked = !!k.value;
      el.addEventListener('change', () => send(k.id, el.checked));
    } else if (k.kind === 'slider') {
      el = document.createElement('input');
      el.type = 'range';
      el.min = k.min ?? 0;
      el.max = k.max ?? 100;
      el.step = 'any';
      el.value = k.value ?? 0;
      let key = nextKey('tsl');
      el.addEventListener('input', () => send(k.id, +el.value, key));
      el.addEventListener('change', () => { key = nextKey('tsl'); });
    } else if (k.kind === 'font') {
      el = document.createElement('select');
      const fill = fonts => {
        const cur = (k.value && k.value.family) || '';
        el.innerHTML = '';
        for (const f of new Set([cur, ...fonts].filter(Boolean))) { const o = document.createElement('option'); o.textContent = f; el.appendChild(o); }
        el.value = cur;
      };
      fill(fontList || ['Inter', 'Noto Serif', 'JetBrains Mono']);
      if (!fontList) invoke('fonts').then(l => { fontList = l; fill(l); }).catch(() => {});
      el.addEventListener('change', () => send(k.id, { family: el.value }));
    } else continue;
    el.id = `prop-title-${k.id}`;
    if (row.className !== 'check') row.appendChild(el);
    box.appendChild(row);
  }
}
/** Saves a text clip as one of the user's titles, named by its first line. */
async function saveAsTitle(c) {
  const name = ((c.props.text && c.props.text.text) || 'My title').split('\n')[0].trim().slice(0, 40) || 'My title';
  try { await invoke('save_title', { clip: c.id, name }); } catch (e) { toast(e, true); return; }
  TI.thumbs.clear();
  toast(`Saved as a title: “${name}”. It is in the Text tab, under My Titles.`);
  if (!$('#panel-text').hidden) renderTitles(true); else TI.list = null;
}

/* ---------- text animations (in and out) and music that ducks under speech ---------- */
const AN = { list: null, timer: 0 };
invoke('text_animations').then(l => { AN.list = l; propsSig = ''; if (has()) renderProps(); }).catch(() => {});
/** Plays a text clip's animation once, so the choice can be seen. */
async function previewAnimation(id, enter) {
  const c = clipById(id), a = c && c.props.text && c.props.text.animations[enter ? 'in' : 'out'];
  if (!a) return;
  clearTimeout(AN.timer);
  const from = enter ? c.start : Math.max(c.start, c.end - a.seconds - 0.4);
  const to = enter ? Math.min(c.end, c.start + a.seconds + 0.4) : c.end;
  await seek(from);
  await play();
  AN.timer = setTimeout(() => { if (S.playing) pause(); }, (to - from) * 1000);
}
/** The Animation section: how a text comes in and goes out, and how long each takes. */
function animationControls(s, ids) {
  if (!AN.list) { s.innerHTML += '<p class="hint">Loading the animations…</p>'; return; }
  for (const [side, enter, label] of [['in', true, 'In'], ['out', false, 'Out']]) {
    const now = c => c.props.text && c.props.text.animations[side];
    const head = document.createElement('div');
    head.className = 'prop wide lead';
    head.innerHTML = `<span>${label}</span>`;
    const opts = [{ id: null, name: 'None' }].concat(AN.list[side]);
    const seg = buttons(opts.map(o => [o.name, o.id ? `${enter ? 'Come in' : 'Go out'} with ${o.name.toLowerCase()}` : `No animation ${enter ? 'in' : 'out'}`, async () => {
      const c = clipById(ids[0]), cur = c && now(c);
      const tl = await edit('set_text_animation', { clips: ids, enter, id: o.id, seconds: cur ? cur.seconds : AN.list.seconds });
      if (tl && o.id) previewAnimation(ids[0], enter);
    }]));
    seg.id = `prop-anim-${side}`;
    head.appendChild(seg);
    controls.push({ el: seg, read: c => { const a = now(c); return a ? a.id : null; }, write: (el, v) => { [...el.children].forEach((b, i) => b.setAttribute('aria-pressed', String(opts[i].id === v))); } });
    s.appendChild(head);
    const row = document.createElement('label');
    row.className = 'prop';
    row.innerHTML = `<span>Length</span><input type="range" min="0.1" max="3" step="0.05"><input type="number" step="0.1" title="seconds">`;
    const [range, num] = row.querySelectorAll('input');
    range.id = `prop-anim-${side}-length`;
    num.dataset.dp = '2';
    let k = nextKey(`anim${side}`);
    const send = (v, merge) => {
      const c = clipById(ids[0]), cur = c && now(c);
      num.value = v;
      if (cur) edit('set_text_animation', { clips: ids, enter, id: cur.id, seconds: +v, merge });
    };
    range.addEventListener('input', () => send(range.value, k));
    range.addEventListener('change', () => { k = nextKey(`anim${side}`); range.blur(); previewAnimation(ids[0], enter); });
    num.addEventListener('change', () => { if (+num.value > 0) { range.value = num.value; send(+num.value, null); } });
    const read = c => { const a = now(c); return a ? a.seconds : AN.list.seconds; };
    controls.push({ el: range, read }, { el: num, read });
    s.appendChild(row);
  }
}
/** Ducking: music on a lane goes quieter while people talk. */
async function setDucking(ids, on, reduceDb) {
  const r = await run('set_ducking', { clips: ids, on, reduceDb });
  if (!r) { propsSig = ''; renderProps(); return; }
  apply(r.timeline);
  if (on) toast(r.dips ? `The music goes down under speech in ${r.dips} ${r.dips === 1 ? 'place' : 'places'}` : 'There is no speech under this music to duck under');
}
function duckingControls(s, ids) {
  const duck = c => (soundOf(c) || c).props.ducking;
  const row = document.createElement('label');
  row.className = 'prop wide';
  row.innerHTML = '<span>Ducking</span><span class="check"><input type="checkbox" id="prop-duck"> Lower under speech</span>';
  const box = row.querySelector('input');
  const amount = document.createElement('label');
  amount.className = 'prop';
  amount.innerHTML = '<span>Lower by</span><input type="range" min="-40" max="-3" step="1"><input type="number" step="1" title="dB">';
  const [range, num] = amount.querySelectorAll('input');
  range.id = 'prop-duck-db';
  num.dataset.dp = '0';
  const want = () => clamp(+range.value || -15, -40, -3);
  const again = buttons([['Update', 'Find the speech again (after moving or trimming clips)', () => setDucking(ids, true, want())]]);
  box.addEventListener('change', () => { amount.hidden = again.hidden = !box.checked; setDucking(ids, box.checked, want()); });
  range.addEventListener('input', () => { num.value = range.value; });
  range.addEventListener('change', () => { range.blur(); if (box.checked) setDucking(ids, true, want()); });
  num.addEventListener('change', () => { range.value = num.value; if (box.checked) setDucking(ids, true, want()); });
  const read = c => { const d = duck(c); return d == null ? -15 : d; };
  // how far, and Update, only while it ducks
  const shown = on => { box.checked = on; amount.hidden = again.hidden = !on; };
  controls.push({ el: box, read: c => duck(c) != null, write: (_, on) => shown(on) }, { el: range, read }, { el: num, read });
  s.appendChild(row);
  s.appendChild(amount);
  s.appendChild(again);
}

/** Beats: marked on music, shown on the clip, snapped to. */
function beatControls(s, ids) {
  const row = document.createElement('div');
  row.className = 'prop wide lead';
  row.innerHTML = '<span>Beats</span>';
  const snd = () => { const c = clipById(ids[0]); return c && (soundOf(c) || c); };
  row.appendChild(buttons([
    ['Mark beats', 'Find the beats of this music and mark them on the clip (things then snap to them)', async () => {
      const r = await run('mark_beats', { clip: ids[0] });
      if (!r) return;
      apply(r.timeline);
      toast(`${r.count} beats marked, ${Math.round(r.bpm)} a minute`);
    }],
    ['Beat at playhead', 'Add a beat at the playhead, or take away the one there', () => edit('toggle_beat', () => ({ clip: ids[0], seconds: S.t }))],
    ['Clear', 'Take every beat off this clip', () => { const c = snd(); if (c && c.beats.length) edit('clear_beats', { clip: ids[0] }); }],
  ]));
  s.appendChild(row);
}
/* ---------- precise trimming: edges to the playhead, slips, nudges, moving a cut ---------- */
const hasMedia = c => c && c.kind !== 'text' && c.kind !== 'image' && !isCap(c);
/** Clip `c`'s start or end to the playhead. */
function trimToPlayhead(c, edgeIn) {
  return edit('trim_to', () => ({ clip: c.id, edgeIn, seconds: S.t }));
}
/** The selected clip shows media `n` frames later (earlier when negative), in the same place. */
async function slipSel(n) {
  const c = selectedClips().length === 1 ? selectedClips()[0] : null;
  if (!hasMedia(c)) { toast('Select one video or sound clip to slip it.'); return; }
  await edit('slip', { clip: c.id, frames: n });
}
/** The selected lane clips `n` frames later (earlier when negative). */
async function nudgeSel(n) {
  const sel = selectedClips().filter(c => !isCap(c));
  if (!sel.length) { toast('Select a clip to move it a frame at a time.'); return; }
  await edit('nudge', { clips: sel.map(c => c.id), frames: n });
}
/** The cut after main-track clip `from`, `n` frames later (earlier when negative). */
async function rollCut(from, n) {
  await edit('roll', { from, frames: n });
}
const NUDGE = '(, and . ; with Shift, 10 frames)';
function trimControls(s, c) {
  s.appendChild(buttons([
    ['Start to playhead', 'Trim the start of the clip to the playhead', () => trimToPlayhead(c, true)],
    ['End to playhead', 'Trim the end of the clip to the playhead', () => trimToPlayhead(c, false)],
  ]));
  if (hasMedia(c)) {
    const row = document.createElement('div');
    row.className = 'prop wide';
    row.innerHTML = '<span title="Other frames of the media, in the same place and length">Slip</span>';
    row.appendChild(buttons([
      ['◀ 1 frame', 'Show the media a frame earlier (Alt+,)', () => slipSel(-1)],
      ['1 frame ▶', 'Show the media a frame later (Alt+.)', () => slipSel(1)],
    ]));
    s.appendChild(row);
  }
  if (!isMain(c)) {
    const row = document.createElement('div');
    row.className = 'prop wide';
    row.innerHTML = '<span>Move</span>';
    row.appendChild(buttons([
      ['◀ 1 frame', `Move the clip a frame earlier ${NUDGE}`, () => nudgeSel(-1)],
      ['1 frame ▶', `Move the clip a frame later ${NUDGE}`, () => nudgeSel(1)],
    ]));
    s.appendChild(row);
  }
}
/** Moving the selected cut: one clip grows as the other shrinks. */
function rollControls(box, from) {
  const row = document.createElement('div');
  row.className = 'prop wide';
  row.innerHTML = '<span title="One clip grows as the other shrinks, so the video keeps its length">Move cut</span>';
  row.appendChild(buttons([
    ['◀ 1 frame', `Move the cut a frame earlier ${NUDGE}`, () => rollCut(from, -1)],
    ['1 frame ▶', `Move the cut a frame later ${NUDGE}`, () => rollCut(from, 1)],
  ]));
  box.appendChild(row);
}

let propsSig = '';
const controls = [];   // {el, read: clip → value, write?(el, v)}
function renderProps() {
  const box = $('#props'), sel = selectedClips();
  if (S.cut != null) {
    const t = transitionAt(S.cut), sig = `cut:${S.cut}:${t ? t.id + ':' + t.max : ''}`;
    $('#props-title').textContent = t ? 'Transition' : 'Cut';
    if (sig === propsSig) { syncTransition(); return; }
    propsSig = sig;
    box.innerHTML = '';
    controls.length = 0;
    transitionControls(box, t);
    rollControls(box, S.cut);
    syncTransition();
    return;
  }
  const one = sel.length === 1 ? sel[0] : null;
  const sig = one ? `${one.id}:${one.kind}:${isCap(one) || isMain(one)}:${!isCap(one) && hasSound(one)}:${keyTimes(one).length > 0}:${pictureClip(one)}:${S.kf && S.kf.clip === one.id ? S.kf.t : ''}` : `n${sel.length}`;
  $('#props-title').textContent = one ? (one.graphic ? 'Graphic' : one.title ? 'Title' : { text: 'Text', audio: 'Sound', image: 'Picture', caption: 'Caption' }[one.kind] || (isMain(one) ? 'Clip' : 'Overlay')) : sel.length ? `${sel.length} clips` : 'Project';
  if (sig === propsSig) { syncProps(one); return; }
  propsSig = sig;
  box.innerHTML = '';
  controls.length = 0;
  if (!one) return;
  const ids = [one.id];
  if (isCap(one)) { captionControls(box, one); syncProps(one); return; }
  // a section named like the panel needs no heading of its own
  const section = title => {
    const s = document.createElement('section');
    if (title.toLowerCase() !== $('#props-title').textContent.toLowerCase()) s.innerHTML = `<h3>${title}</h3>`;
    box.appendChild(s);
    return s;
  };
  if (one.graphic) {
    const s = section('Graphic');
    s.appendChild(buttons([[`Edit in Graphics`, `Open this ${(TOOL_NAMES[one.graphic] || 'graphic').toLowerCase()} in Graphics to change it`, () => openGraphics(one.id)]]));
  }
  if (one.title) titleControls(section('Title'), one);
  if (one.kind === 'text' && !one.title) {
    const t = section('Text');
    textControls(t, ids);
    t.appendChild(buttons([['Save as title', 'Keep this text, its look and its animations as a title to use again (Text tab, My Titles)', () => saveAsTitle(clipById(one.id))]]));
    animationControls(section('Animation'), ids);
  }
  if (visual(one)) {
    const s = section('Picture');
    s.appendChild(slider('Scale', ids, 'scale', { min: 5, max: 400, step: 1, unit: '%', read: c => c.props.scale, kf: 'scale' }));
    s.appendChild(pair('Position', ids, [['x', c => c.props.x], ['y', c => c.props.y]], 'position'));
    s.appendChild(slider('Rotation', ids, 'rotation', { min: -180, max: 180, step: 1, unit: '°', read: c => c.props.rotation, kf: 'rotation' }));
    s.appendChild(slider('Opacity', ids, 'opacity', { min: 0, max: 100, step: 1, unit: '%', read: c => c.props.opacity, kf: 'opacity' }));
    if (one.kind !== 'text') {
      s.appendChild(buttons([
        ['Fit', 'Show the whole picture', () => edit('fit_clips', { clips: ids, fill: false })],
        ['Fill', 'Fill the frame, cropping the edges', () => edit('fit_clips', { clips: ids, fill: true })],
        ['Centre', 'Back to the middle, full size', () => queueProps(ids, { x: S.tl.width / 2, y: S.tl.height / 2, scale: 100, rotation: 0 })],
      ]));
    }
  }
  if (hasSound(one)) {
    const s = section('Sound');
    s.appendChild(slider('Volume', ids, 'volumeDb', { min: -60, max: 15, step: 0.5, unit: 'dB', read: c => (c.live && c.live.volumeDb != null ? c.live.volumeDb : (soundOf(c) || c).props.volumeDb), kf: 'volume' }));
    // fades of the clip's own sound (a main clip's too: its picture's edges are the transitions')
    s.appendChild(slider('Fade in', ids, 'fadeIn', { min: 0, max: 10, step: 0.1, unit: 's', read: c => (soundOf(c) || c).props.fadeIn }));
    s.appendChild(slider('Fade out', ids, 'fadeOut', { min: 0, max: 10, step: 0.1, unit: 's', read: c => (soundOf(c) || c).props.fadeOut }));
    if (!isMain(one)) {
      duckingControls(s, ids);
      beatControls(s, ids);
    }
    // a picture's own sound can become a clip of its own on a sound lane
    const pic = one.track[0] === 'V' ? one : one.link && S.tl.clips.find(v => v.link === one.link && v.track[0] === 'V');
    if (pic && ids.length === 1) {
      s.appendChild(buttons([['Extract audio', 'Put this clip’s sound on a sound lane as a clip of its own, to trim, move and fade it apart from the picture', () => make('extract_audio', { clip: one.id })]]));
      s.lastChild.firstChild.id = 'extract-audio';
    }
  }
  if (visual(one) || hasSound(one)) keyframeControls(section('Keyframes'), one);
  trimControls(section('Trim'), one);
  if (pictureClip(one)) adjustControls(box, ids);
  if (one.kind !== 'text' && one.kind !== 'image') {
    const s = section('Speed');
    const presets = [50, 100, 150, 200, 300];
    const seg = buttons(presets.map(p => [`${p / 100}×`, `Play at ${p}%`, () => queueProps(ids, { speed: p })]));
    s.appendChild(seg);
    controls.push({ el: seg, read: c => c.props.speed, write: (el, v) => { [...el.children].forEach((b, i) => b.setAttribute('aria-pressed', String(Math.abs(presets[i] - v) < 0.5))); } });
    s.appendChild(slider('Speed', ids, 'speed', { min: 10, max: 400, step: 5, unit: '%', read: c => c.props.speed, live: false }));
  }
  syncProps(one);
}
/** A caption's words and times (its look is the Captions tab's, for all of them). */
function captionControls(box, c) {
  const ta = document.createElement('textarea');
  ta.id = 'prop-caption';
  ta.setAttribute('aria-label', 'Caption');
  ta.addEventListener('keydown', e => e.stopPropagation());
  ta.addEventListener('change', () => edit('set_caption_text', { id: c.id, text: ta.value }));
  controls.push({ el: ta, read: x => x.text });
  box.appendChild(ta);
  const times = document.createElement('div');
  times.className = 'prop';
  times.innerHTML = '<span>From, to</span><input type="number" step="0.1" title="Starts at, seconds"><input type="number" step="0.1" title="Ends at, seconds">';
  const [a, b] = times.querySelectorAll('input');
  a.style.gridColumn = '2';
  const send = () => { if (Number.isFinite(+a.value) && Number.isFinite(+b.value)) edit('retime_caption', { id: c.id, start: +a.value, end: +b.value }); };
  a.addEventListener('change', send);
  b.addEventListener('change', send);
  a.dataset.dp = b.dataset.dp = '2';
  controls.push({ el: a, read: x => x.start }, { el: b, read: x => x.end });
  box.appendChild(times);
  box.appendChild(buttons([
    ['Go to it', 'Move the playhead to this caption', () => { seek(c.start); scrollToTime(c.start); }],
    ['Delete', 'Delete this caption', () => edit('delete_captions', { ids: [c.id] })],
    ['Captions tab', 'Change how all the captions look', () => showTab('captions')],
  ]));
}
function syncTransition() {
  for (const k of controls) {
    if (k.el === document.activeElement) continue;
    const v = k.read();
    if (k.write) k.write(k.el, v); else k.el.value = v;
  }
}
function syncProps(one) {
  if (!one) return;
  const c = liveClip(clipById(one.id));
  if (!c) return;
  for (const k of controls) {
    if (k.el.contains(document.activeElement) && document.activeElement !== document.body && !k.el.matches('.seg')) continue;
    const v = k.read(c);
    if (k.write) k.write(k.el, v);
    else if (k.el.type === 'checkbox') k.el.checked = !!v;
    else k.el.value = v == null ? '' : (typeof v === 'number' ? +v.toFixed(k.el.dataset.dp ? +k.el.dataset.dp : 1) : v);
  }
}
/** A labelled slider with a number box; dragging it is one undo step. */
function slider(label, ids, key, { min, max, step, unit, read, live = true, kf = null }) {
  const row = document.createElement('label');
  row.className = 'prop';
  row.innerHTML = `<span>${label}</span><input type="range" min="${min}" max="${max}" step="${step}"><input type="number" step="${step}" title="${unit}">`;
  if (kf) row.firstChild.prepend(kfButton(ids, kf));
  const [range, num] = row.querySelectorAll('input');
  range.id = `prop-${key}`;
  let k = nextKey(key);
  const send = (v, merge) => { num.value = v; queueProps(ids, { [key]: +v }, merge); };
  range.addEventListener('input', () => { if (live) send(range.value, k); else num.value = range.value; });
  range.addEventListener('change', () => { if (!live) send(range.value, null); k = nextKey(key); range.blur(); });
  num.addEventListener('change', () => { const v = clamp(+num.value, -1e6, 1e6); if (Number.isFinite(v)) { range.value = v; send(v, null); } });
  controls.push({ el: range, read }, { el: num, read });
  return row;
}
/** Two number boxes on one row (position X and Y). */
function pair(label, ids, keys, kf = null) {
  const row = document.createElement('div');
  row.className = 'prop';
  row.innerHTML = `<span>${label}</span><input type="number" step="1" title="X, canvas pixels"><input type="number" step="1" title="Y, canvas pixels">`;
  if (kf) row.firstChild.prepend(kfButton(ids, kf));
  row.querySelectorAll('input').forEach((inp, i) => {
    const [key, read] = keys[i];
    inp.dataset.dp = '0';
    inp.addEventListener('change', () => { if (Number.isFinite(+inp.value)) queueProps(ids, { [key]: +inp.value }); });
    controls.push({ el: inp, read });
  });
  row.querySelectorAll('input')[0].style.gridColumn = '2';
  return row;
}
function buttons(list) {
  const seg = document.createElement('div');
  seg.className = 'seg';
  for (const [label, title, fn] of list) {
    const b = document.createElement('button');
    b.textContent = label;
    b.title = title;
    b.addEventListener('click', fn);
    seg.appendChild(b);
  }
  return seg;
}
let fontList = null;
function textControls(s, ids) {
  const t = c => c.props.text || {};
  const ta = document.createElement('textarea');
  ta.id = 'prop-text';
  ta.setAttribute('aria-label', 'Text');
  ta.spellcheck = true;
  let typing = 0;
  ta.addEventListener('input', () => { clearTimeout(typing); typing = setTimeout(() => queueProps(ids, { text: ta.value }, `text:${ids}`), 250); });
  ta.addEventListener('keydown', e => e.stopPropagation());
  controls.push({ el: ta, read: c => t(c).text });
  s.appendChild(ta);
  // font
  const fr = document.createElement('label');
  fr.className = 'prop wide';
  fr.innerHTML = '<span>Font</span><select id="prop-font"></select>';
  const sel = fr.querySelector('select');
  const fill = list => {
    const cur = sel.value;
    sel.innerHTML = '';
    for (const f of list) { const o = document.createElement('option'); o.value = o.textContent = f; sel.appendChild(o); }
    if (cur) sel.value = cur;
  };
  fill(fontList || ['Inter', 'Noto Serif', 'JetBrains Mono']);
  if (!fontList) invoke('fonts').then(list => { fontList = list; fill(list); syncProps(clipById(ids[0])); }).catch(() => {});
  sel.addEventListener('change', () => { queueProps(ids, { font: sel.value }); sel.blur(); });
  controls.push({ el: sel, read: c => t(c).font, write: (el, v) => { if (v && ![...el.options].some(o => o.value === v)) { const o = document.createElement('option'); o.value = o.textContent = v; el.appendChild(o); } el.value = v || 'Inter'; } });
  s.appendChild(fr);
  s.appendChild(slider('Size', ids, 'size', { min: 8, max: 400, step: 1, unit: 'px', read: c => t(c).size }));
  // style and alignment
  const style = buttons([
    ['Bold', 'Bold', () => queueProps(ids, { bold: !t(clipById(ids[0])).bold })],
    ['Italic', 'Italic', () => queueProps(ids, { italic: !t(clipById(ids[0])).italic })],
  ]);
  controls.push({ el: style, read: c => t(c), write: (el, v) => { el.children[0].setAttribute('aria-pressed', String(!!v.bold)); el.children[1].setAttribute('aria-pressed', String(!!v.italic)); } });
  const align = buttons(['left', 'center', 'right'].map(a => [cap(a === 'center' ? 'centre' : a), `Align ${a === 'center' ? 'centre' : a}`, () => queueProps(ids, { align: a })]));
  controls.push({ el: align, read: c => t(c).align, write: (el, v) => { ['left', 'center', 'right'].forEach((a, i) => el.children[i].setAttribute('aria-pressed', String(a === v))); } });
  const both = document.createElement('div');
  both.className = 'prop wide';
  both.innerHTML = '<span>Style</span>';
  const pair2 = document.createElement('div');
  pair2.style.cssText = 'display:flex;gap:8px;flex-wrap:wrap';
  pair2.append(style, align);
  both.appendChild(pair2);
  s.appendChild(both);
  // colours, outline, shadow, box
  s.appendChild(colorRow('Colour', ids, null, 'color', c => t(c).color));
  s.appendChild(colorRow('Outline', ids, ['stroke', c => t(c).stroke], 'strokeColor', c => t(c).strokeColor));
  s.appendChild(slider('Width', ids, 'strokeWidth', { min: 0, max: 40, step: 1, unit: 'px', read: c => t(c).strokeWidth }));
  s.appendChild(colorRow('Box', ids, ['background', c => t(c).background], 'backgroundColor', c => t(c).backgroundColor));
  const sh = document.createElement('label');
  sh.className = 'check';
  sh.innerHTML = '<input type="checkbox" id="prop-shadow"> Shadow';
  const shc = sh.querySelector('input');
  shc.addEventListener('change', () => queueProps(ids, { shadow: shc.checked }));
  controls.push({ el: shc, read: c => t(c).shadow });
  s.appendChild(sh);
}
/** A colour, with an on/off switch in front when `toggle` is given. */
function colorRow(label, ids, toggle, key, read) {
  const row = document.createElement('div');
  row.className = 'prop wide';
  row.innerHTML = `<span>${label}</span><div class="check"></div>`;
  const box = row.querySelector('.check');
  if (toggle) {
    const cb = document.createElement('input');
    cb.type = 'checkbox';
    cb.title = `${label} on or off`;
    cb.addEventListener('change', () => queueProps(ids, { [toggle[0]]: cb.checked }));
    controls.push({ el: cb, read: toggle[1] });
    box.appendChild(cb);
  }
  const col = document.createElement('input');
  col.type = 'color';
  col.title = label;
  let k = nextKey(key);
  col.addEventListener('input', () => queueProps(ids, toggle ? { [key]: col.value, [toggle[0]]: true } : { [key]: col.value }, k));
  col.addEventListener('change', () => { k = nextKey(key); });
  controls.push({ el: col, read });
  box.appendChild(col);
  return row;
}

function renderDetails() {
  let rows;
  const sel = selectedClips();
  const cut = S.cut != null && mainCuts().find(c => c.from === S.cut);
  if (!has()) rows = [['Project', projectName()], ['Clips', 'None yet']];
  else if (cut) {
    const t = transitionAt(cut.from), name = id => { const c = clipById(id); return c ? c.name : ''; };
    rows = [['Transition', t ? t.name : 'None'], ['Cut at', fmtFrames(cut.at)], ['From', name(cut.from)], ['Into', name(cut.to)]];
    if (t) rows.push(['Plays', `${tcShort(t.start)} – ${tcShort(t.end)}`], ['Length', fmtLen(t.end - t.start)]);
  } else if (sel.length === 1 && isCap(sel[0])) {
    const c = sel[0];
    rows = [['Caption', `${tcShort(c.start)} – ${tcShort(c.end)}`], ['Length', fmtLen(c.end - c.start)], ['Speaker', c.speaker || '—']];
  } else if (sel.length === 1) {
    const c = sel[0], m = mediaOf(c.item);
    rows = [
      ['Clip', c.kind === 'text' ? 'Text' : c.name],
      ['Starts at', fmtFrames(c.start)],
      ['Length', fmtFrames(c.end - c.start)],
    ];
    if (c.kind !== 'text') {
      rows.push(['From the media', `${tcShort(c.sourceIn)} – ${tcShort(c.sourceOut)}`], ['Media length', fmtFrames(c.mediaDuration)]);
      if (c.kind !== 'audio') rows.push(['Sound', hasSound(c) ? 'Its own' : 'None']);
      rows.push(['File', m ? m.path : '']);
    }
  } else if (sel.length > 1) {
    rows = [['Selected', `${sel.length} clips`], ['Total length', fmtFrames(sel.reduce((n, c) => n + c.end - c.start, 0))]];
  } else {
    rows = [
      ['Project', projectName()],
      ['Canvas', `${S.tl.width} × ${S.tl.height} (${ratioName(S.tl.width, S.tl.height)})`],
      ['Frame rate', `${+S.tl.fps.toFixed(3)} fps`],
      ['Length', fmtFrames(dur())],
      ['Clips', String(mainClips().length)],
      ['Captions', S.tl.captions.exists ? String(S.tl.captions.items.length) : 'None'],
      ['Saved in', S.tl.project.path || 'Not saved yet'],
    ];
  }
  const dl = $('#details');
  dl.innerHTML = rows.map(() => '<div><dt></dt><dd></dd></div>').join('');
  [...dl.children].forEach((d, k) => {
    d.firstChild.textContent = rows[k][0];
    const path = /^(File|Saved in)$/.test(rows[k][0]);
    // a path is cut at its start; the marks keep it reading left to right inside the rtl box
    d.lastChild.textContent = path ? `‎${rows[k][1]}‎` : rows[k][1];
    d.lastChild.title = rows[k][1];
    d.lastChild.classList.toggle('path', path);
  });
}

/* ---------- editing ---------- */
async function split() {
  if (!has()) return;
  await edit('split', () => ({ seconds: S.t, selection: [...S.sel] }));
}
async function splitKeep(keep) {
  if (!has()) return;
  const r = await run('split_keep', () => ({ seconds: S.t, selection: [...S.sel], keep }));
  if (!r) return;
  S.t = r.playhead;
  apply(r.timeline);
}
async function deleteSelection() {
  if (S.kf) { await deleteKeyframe(); return; }
  if (transitionAt(S.cut)) { await edit('remove_transitions', { from: [S.cut] }); return; }
  if (!S.sel.size) { toast('Select a clip to delete'); return; }
  const caps = new Set(capClips().map(c => c.id));
  const c = [...S.sel].filter(id => caps.has(id)), v = [...S.sel].filter(id => !caps.has(id));
  if (c.length) await edit('delete_captions', { ids: c });
  if (v.length) await edit('delete_clips', { clips: v });
}
const undo = () => edit('undo');
const redo = () => edit('redo');
function toggleSnap() {
  S.snap = !S.snap;
  renderTools();
  toast(S.snap ? 'Snapping on' : 'Snapping off');
}

/* ---------- dragging ---------- */
/** Follows the pointer until it is released (or Esc), scrolling the timeline when it nears an
    edge. `move(ev)` gets the pointer (and runs again as the timeline scrolls); `end(cancelled)`. */
function follow(e, move, end, { scroll = true } = {}) {
  const sc = scroller();
  let last = { clientX: e.clientX, clientY: e.clientY, shiftKey: e.shiftKey };
  let raf = 0, done = false;
  S.dragging = true;
  const tick = () => {
    if (done) return;
    const r = sc.getBoundingClientRect();
    const inRows = last.clientY > r.top - 60 && last.clientY < r.bottom + 60;
    let d = 0;
    if (inRows && last.clientX < r.left + EDGE_PX) d = -EDGE_STEP;
    else if (inRows && last.clientX > r.right - EDGE_PX) d = EDGE_STEP;
    if (d) {
      const before = sc.scrollLeft;
      sc.scrollLeft += d;
      if (sc.scrollLeft !== before) { drawVisible(); move(last); }
    }
    raf = requestAnimationFrame(tick);
  };
  const onMove = ev => {
    last = { clientX: ev.clientX, clientY: ev.clientY, shiftKey: ev.shiftKey };
    if (scroll && !raf) raf = requestAnimationFrame(tick);
    move(last);
  };
  const onKey = ev => {
    if (ev.key === 'Escape') { ev.preventDefault(); ev.stopPropagation(); finish(true); }
    else if (ev.key === 'Shift') { last = { ...last, shiftKey: ev.type === 'keydown' }; move(last); }
  };
  const onUp = () => finish(false);
  function finish(cancelled) {
    if (done) return;
    done = true;
    cancelAnimationFrame(raf);
    window.removeEventListener('pointermove', onMove);
    window.removeEventListener('pointerup', onUp);
    window.removeEventListener('keydown', onKey, true);
    window.removeEventListener('keyup', onKey, true);
    S.dragging = false;
    hideSnap();
    end(cancelled, last);
  }
  window.addEventListener('pointermove', onMove);
  window.addEventListener('pointerup', onUp);
  window.addEventListener('keydown', onKey, true);
  window.addEventListener('keyup', onKey, true);
}
/** Pulls `t` to the nearest of `targets` within reach, unless snapping is off or Shift is held. */
function snapTo(t, ev, targets) {
  const best = nearest(t, ev, targets);
  if (best === null) { hideSnap(); return t; }
  showSnap(best);
  return best;
}
function nearest(t, ev, targets) {
  if (!S.snap || ev.shiftKey) return null;
  let best = null, reach = SNAP_PX / S.px;
  for (const x of targets) {
    const d = Math.abs(x - t);
    if (d <= reach) { reach = d; best = x; }
  }
  return best;
}
function showSnap(t) {
  const line = $('#snap-line');
  line.style.left = (PAD + t * S.px) + 'px';
  line.hidden = false;
}
function hideSnap() { $('#snap-line').hidden = true; }
/** Every cut on the main track, and its ends. */
function cuts() {
  const set = new Set([0]);
  for (const c of mainClips()) { set.add(+c.start.toFixed(6)); set.add(+c.end.toFixed(6)); }
  return [...set].sort((a, b) => a - b);
}
/** Where things can snap to: the playhead, the main track's cuts, every other clip's edges, beats. */
function snapTargets(except) {
  const out = [S.t, ...cuts()];
  for (const c of visibleClips()) if (!except.has(c.id)) out.push(c.start, c.end);
  // and the beats of music (of a picture clip's own sound too)
  for (const c of S.tl.clips) if (c.beats && !except.has(c.id) && !except.has((visibleClips().find(v => v.link && v.link === c.link) || {}).id)) out.push(...c.beats);
  return out;
}

/* Scrubbing: the ruler, or the timeline outside the clips */
function scrubFrom(e, deselect) {
  if (!has() || e.button !== 0) return;
  if (deselect && (S.sel.size || S.cut != null)) select([]);
  const at = ev => seek(snapTo(timeAt(ev.clientX), ev, cuts()));
  at(e);
  follow(e, at, () => {});
}

/* Clips: click selects (Ctrl or Shift adds), a drag moves (main clips reorder), the edges trim */
function clipPointerDown(c, el, e) {
  if (e.button !== 0) return;
  e.stopPropagation();
  e.preventDefault();
  const handle = e.target.closest('.handle');
  if (handle) { (isCap(c) ? captionTrim : startTrim)(c, handle.classList.contains('in') ? 'in' : 'out', e); return; }
  const add = e.ctrlKey || e.metaKey || e.shiftKey;
  const wasSelected = S.sel.has(c.id);
  if (add) {
    const ids = new Set(S.sel);
    if (wasSelected) ids.delete(c.id); else ids.add(c.id);
    select(ids);
    return;
  }
  if (!wasSelected) select([c.id]);
  const after = started => {
    // a plain click on a clip in a group selects just that clip
    if (!started && wasSelected && S.sel.size > 1) select([c.id]);
  };
  if (isCap(c)) captionDrag(c, el, e, after);
  else if (isMain(c)) reorderDrag(c, el, e, after);
  else laneDrag(c, el, e, after);
}
function reorderDrag(c, el, e, after) {
  const x0 = e.clientX;
  let started = false;
  const grab = timeAt(e.clientX) - c.start;
  const len = c.end - c.start;
  const others = mainClips().filter(o => o.id !== c.id);
  // the others' places without this clip: where its centre lands among them sets its new place
  const packed = [];
  { let at = 0; for (const o of others) { packed.push({ o, start: at, end: at + o.end - o.start }); at += o.end - o.start; } }
  const from = mainClips().findIndex(o => o.id === c.id);
  let to = from;
  follow(e, ev => {
    if (!started) {
      if (Math.abs(ev.clientX - x0) < 5) return;
      started = true;
      el.classList.add('dragging');
      for (const o of others) S.els.get(o.id)?.classList.add('moving');
    }
    const start = Math.max(0, timeAt(ev.clientX) - grab);
    const centre = start + len / 2;
    to = packed.filter(p => (p.start + p.end) / 2 < centre).length;
    // the others make room where it would go
    S.geom.clear();
    let at = 0;
    packed.forEach((p, k) => {
      if (k === to) at += len;
      S.geom.set(p.o.id, { ...p.o, start: at, end: at + p.o.end - p.o.start });
      at += p.o.end - p.o.start;
    });
    S.geom.set(c.id, { ...c, start, end: start + len });
    const gap = packed.slice(0, to).reduce((n, p) => n + p.end - p.start, 0);
    const mark = $('#insert-mark');
    mark.style.left = (PAD + gap * S.px) + 'px';
    mark.hidden = false;
    layoutClips();
    drawVisible();
  }, cancelled => {
    $('#insert-mark').hidden = true;
    el.classList.remove('dragging');
    for (const o of others) S.els.get(o.id)?.classList.remove('moving');
    S.geom.clear();
    layoutClips();
    drawVisible();
    after(started);
    if (started && !cancelled && to !== from) edit('reorder', { clip: c.id, to });
  });
}
/** The lane a lane clip would go to with the pointer at `y`: one of its kind's lanes, or a new
    one past the far edge (above the top overlay lane, below the last sound lane). */
function laneFor(c, y) {
  const kind = laneKind(c);
  const rows = S.rows.filter(r => r.kind === kind);
  const own = rowOf(c.track);
  if (!rows.length || !own) return { id: c.track, dy: 0 };
  const first = rows[0], last = rows[rows.length - 1];
  if (kind === 'video' && y < first.top - 6) {
    const top = Math.max(...S.tl.lanes.filter(l => l.kind === 'video').map(l => laneNo(l.id)));
    return { id: `V${top + 1}`, dy: first.top - ROW.video - ROW.gap - own.top };
  }
  if (kind === 'audio' && y > last.top + last.h + 6) {
    const bottom = Math.max(...S.tl.lanes.filter(l => l.kind === 'audio').map(l => laneNo(l.id)));
    return { id: `A${bottom + 1}`, dy: last.top + last.h + ROW.gap - own.top };
  }
  let best = own, d = Infinity;
  for (const r of rows) { const dd = Math.abs(y - (r.top + r.h / 2)); if (dd < d) { d = dd; best = r; } }
  return { id: best.id, dy: best.top - own.top };
}
function laneDrag(c, el, e, after) {
  const x0 = e.clientX, y0 = e.clientY;
  let started = false, lane = c.track, start = c.start;
  const grab = timeAt(e.clientX) - c.start, len = c.end - c.start;
  const partner = soundOf(c) && soundOf(c).id !== c.id ? soundOf(c) : (c.link ? visibleClips().find(o => o.link === c.link && o.id !== c.id) : null);
  const moving = new Set([c.id, ...(partner ? [partner.id] : [])]);
  const targets = snapTargets(moving);
  follow(e, ev => {
    if (!started) {
      if (Math.hypot(ev.clientX - x0, ev.clientY - y0) < 5) return;
      started = true;
      el.classList.add('dragging');
    }
    start = Math.max(0, timeAt(ev.clientX) - grab);
    // snap its start or its end, whichever is nearer something
    const a = nearest(start, ev, targets), b = nearest(start + len, ev, targets);
    if (a !== null && (b === null || Math.abs(a - start) <= Math.abs(b - start - len))) { start = a; showSnap(a); }
    else if (b !== null) { start = b - len; showSnap(b); }
    else hideSnap();
    start = Math.max(0, snapFrame(start));
    const to = laneFor(c, yAt(ev.clientY));
    lane = to.id;
    S.geom.set(c.id, { ...c, start, end: start + len, dy: to.dy });
    if (partner) S.geom.set(partner.id, { ...partner, start: partner.start + start - c.start, end: partner.end + start - c.start });
    layoutClips();
    drawVisible();
  }, cancelled => {
    el.classList.remove('dragging');
    S.geom.clear();
    layoutClips();
    drawVisible();
    after(started);
    if (started && !cancelled && (lane !== c.track || Math.abs(start - c.start) > 1e-6)) edit('move_clip', { clip: c.id, lane, seconds: start });
  });
}

/* Captions: dragged along their lane (they then belong to the words there) or trimmed; they
   never cover one another */
function captionRoom(c) {
  const others = capClips().filter(o => o.id !== c.id);
  return {
    prev: Math.max(0, ...others.filter(o => o.end <= c.start + 1e-6).map(o => o.end)),
    next: Math.min(Infinity, ...others.filter(o => o.start >= c.end - 1e-6).map(o => o.start)),
  };
}
function captionDrag(c, el, e, after) {
  const x0 = e.clientX, len = c.end - c.start, grab = timeAt(e.clientX) - c.start;
  const { prev, next } = captionRoom(c);
  const targets = [S.t, ...cuts(), ...visibleClips().flatMap(o => [o.start, o.end])];
  let started = false, start = c.start;
  follow(e, ev => {
    if (!started) { if (Math.abs(ev.clientX - x0) < 5) return; started = true; el.classList.add('dragging'); }
    start = timeAt(ev.clientX) - grab;
    const a = nearest(start, ev, targets), b = nearest(start + len, ev, targets);
    if (a !== null && (b === null || Math.abs(a - start) <= Math.abs(b - start - len))) { start = a; showSnap(a); }
    else if (b !== null) { start = b - len; showSnap(b); } else hideSnap();
    start = snapFrame(clamp(start, prev, Math.max(prev, next - len)));
    S.geom.set(c.id, { ...c, start, end: start + len });
    layoutClips();
  }, cancelled => {
    el.classList.remove('dragging');
    S.geom.clear();
    layoutClips();
    after(started);
    if (started && !cancelled && Math.abs(start - c.start) > 1e-6) edit('retime_caption', { id: c.id, start, end: start + len });
  });
}
function captionTrim(c, edge, e) {
  if (!S.sel.has(c.id)) select([c.id]);
  const f = frameStep(), x0 = e.clientX, { prev, next } = captionRoom(c);
  const targets = [S.t, ...cuts(), ...visibleClips().flatMap(o => [o.start, o.end])];
  let a = c.start, b = c.end;
  follow(e, ev => {
    const t = snapFrame(snapTo((edge === 'in' ? c.start : c.end) + (ev.clientX - x0) / S.px, ev, targets));
    if (edge === 'in') a = clamp(t, prev, c.end - f); else b = clamp(t, c.start + f, next);
    S.geom.set(c.id, { ...c, start: a, end: b });
    layoutClips();
  }, cancelled => {
    S.geom.clear();
    layoutClips();
    if (!cancelled && (a !== c.start || b !== c.end)) edit('retime_caption', { id: c.id, start: a, end: b });
  });
}

/* Trimming an edge: the clip follows (on the main track, later clips with it); the player shows
   the new first or last frame */
function startTrim(c, edge, e) {
  if (!S.sel.has(c.id)) select([c.id]);
  const f = frameStep(), len = c.end - c.start, main = isMain(c);
  const rate = (c.sourceOut - c.sourceIn) / Math.max(1e-6, len);
  // a lane clip stops at its neighbours on the lane; anything stops at its media's ends
  const lane = visibleClips().filter(o => o.track === c.track && o.id !== c.id);
  const prevEnd = main ? 0 : Math.max(0, ...lane.filter(o => o.end <= c.start + 1e-6).map(o => o.end));
  const nextStart = main ? Infinity : Math.min(Infinity, ...lane.filter(o => o.start >= c.end - 1e-6).map(o => o.start));
  const lo = edge === 'out' ? -(len - f) : -Math.min(c.sourceIn / rate, main ? Infinity : c.start - prevEnd);
  const hi = edge === 'out' ? Math.min((c.mediaDuration - c.sourceOut) / rate, nextStart - c.end) : len - f;
  const x0 = e.clientX, s0 = scroller().scrollLeft;
  const later = main ? mainClips().filter(o => o.start >= c.end - 1e-6 && o.id !== c.id) : [];
  const base = edge === 'out' ? c.end : c.start;
  // later main-track cuts move with an end trim, so only the playhead and earlier cuts pull it
  const targets = main ? [S.t, ...(edge === 'out' ? cuts().filter(t => t <= c.start + 1e-6) : cuts())] : snapTargets(new Set([c.id]));
  const tip = document.createElement('div');
  tip.className = 'trim-tip';
  document.body.appendChild(tip);
  let d = 0;
  follow(e, ev => {
    const raw = base + (ev.clientX - x0 + scroller().scrollLeft - s0) / S.px;
    d = snapFrame(Math.min(hi, Math.max(lo, snapTo(raw, ev, targets) - base)));
    if (edge === 'out') {
      S.geom.set(c.id, { ...c, end: c.end + d, sourceOut: c.sourceOut + d * rate });
      for (const o of later) S.geom.set(o.id, { ...o, start: o.start + d, end: o.end + d });
      extraWidth = Math.max(0, d * S.px);
    } else {
      S.geom.set(c.id, { ...c, start: c.start + d, sourceIn: c.sourceIn + d * rate });
    }
    layoutTimeline();
    tip.textContent = `${d >= 0 ? '+' : '−'}${fmtLen(Math.abs(d))} · ${fmtLen(len + (edge === 'out' ? d : -d))}`;
    const r = S.els.get(c.id).getBoundingClientRect();
    tip.style.left = (edge === 'out' ? r.right : r.left) + 'px';
    tip.style.top = r.top + 'px';
    if (c.kind === 'video' || c.kind === 'image') show({ item: c.item, t: edge === 'out' ? c.sourceOut + d * rate - f : c.sourceIn + d * rate });
  }, cancelled => {
    tip.remove();
    S.geom.clear();
    extraWidth = 0;
    layoutTimeline();
    if (!cancelled && Math.abs(d) >= f / 2) edit('trim', { clip: c.id, edge, delta: d });
    else showFrame(S.t);
  });
}

/* A Media card dragged onto the timeline: between main clips on the main track, as an overlay
   above it, and sound below it (anywhere, for a sound card) */
function startMediaDrag(m, e) {
  const x0 = e.clientX, y0 = e.clientY;
  let ghost = null, drop = null;
  const clips = mainClips(), sound = m.kind === 'audio';
  const box = $('#drop-box');
  follow(e, ev => {
    if (!ghost) {
      if (Math.hypot(ev.clientX - x0, ev.clientY - y0) < 6) return;
      ghost = document.createElement('div');
      ghost.className = 'media-ghost' + (sound ? ' sound' : '');
      const src = mediaThumbs.get(m.item);
      if (src) { const c = document.createElement('canvas'); c.width = 120; c.height = 68; drawCover(c.getContext('2d'), src, 0, 0, 120, 68); ghost.appendChild(c); }
      else ghost.innerHTML = `<div class="sound">${ICON.music}</div>`;
      document.body.appendChild(ghost);
    }
    ghost.style.left = ev.clientX + 'px';
    ghost.style.top = ev.clientY + 'px';
    const r = $('.timeline').getBoundingClientRect();
    const over = ev.clientY >= r.top && ev.clientY <= r.bottom && ev.clientX >= r.left && ev.clientX <= r.right;
    S.geom.clear();
    $('#insert-mark').hidden = true;
    box.hidden = true;
    drop = null;
    if (over && has()) {
      const t = Math.max(0, snapFrame(snapTo(timeAt(ev.clientX), ev, snapTargets(new Set()))));
      const y = yAt(ev.clientY), main = mainRow();
      if (sound || y < main.top - 2) {
        // a lane: shown as a box where it will sit
        drop = { kind: 'lane', t };
        const top = sound ? (S.rows.filter(x => x.kind === 'audio').reduce((a, x) => Math.max(a, x.top + x.h), main.top + main.h) + ROW.gap) : Math.max(0, (S.rows[0].kind === 'video' ? S.rows[0].top : main.top) - ROW.video - ROW.gap);
        box.style.left = (PAD + t * S.px) + 'px';
        box.style.width = Math.max(4, m.duration * S.px) + 'px';
        box.style.top = (26 + top) + 'px';
        box.style.height = (sound ? ROW.audio : ROW.video) + 'px';
        box.hidden = false;
      } else {
        const tt = timeAt(ev.clientX);
        const to = clips.filter(c => (c.start + c.end) / 2 < tt).length;
        drop = { kind: 'main', to };
        const at = to < clips.length ? clips[to].start : dur();
        for (const c of clips.slice(to)) S.geom.set(c.id, { ...c, start: c.start + m.duration, end: c.end + m.duration });
        const mark = $('#insert-mark');
        mark.style.left = (PAD + at * S.px) + 'px';
        mark.hidden = false;
      }
    } else if (over) drop = { kind: 'first' };
    layoutClips();
    drawVisible();
  }, cancelled => {
    ghost?.remove();
    $('#insert-mark').hidden = true;
    box.hidden = true;
    S.geom.clear();
    hideSnap();
    layoutClips();
    drawVisible();
    if (cancelled || !ghost || !drop) return;
    if (drop.kind === 'main') edit('insert_media', { item: m.item, index: drop.to });
    else if (drop.kind === 'lane') addToLane(m.item, drop.t);
    // nothing on the timeline yet: a video dropped anywhere on it starts the sequence
    else if (!sound) appendMedia(m.item);
    else edit('start_with_sound', { item: m.item });
  });
}

/* ---------- playback ---------- */
let raf = 0;
function stopLoop() { cancelAnimationFrame(raf); raf = 0; }
async function play() {
  if (!has()) return;
  if (S.t >= dur() - frameStep()) S.t = 0;
  try {
    await invoke('play', { from: S.t, maxWidth: previewWidth(), quality: quality() });
  } catch (e) { toast(e, true); return; }
  S.playing = true;
  setPlayIcon();
  drawOverlay();
  const loop = async () => {
    if (!S.playing) return;
    const st = await invoke('playback');
    if (!S.playing) return;
    S.t = st.seconds;
    $('#sound-note').textContent = st.sound ? '' : 'No sound device';
    $('#sound-note').title = st.sound ? '' : 'There is no sound device to play on, so the video plays without sound. Plug in headphones or speakers, then press Play again.';
    updateTime();
    followPlayhead();
    showFrame(S.t);
    pollMeters();
    if (!st.playing) { S.playing = false; setPlayIcon(); drawOverlay(); settleMeters(); return; }
    raf = requestAnimationFrame(loop);
  };
  raf = requestAnimationFrame(loop);
}
async function pause() {
  stopLoop();
  S.playing = false;
  setPlayIcon();
  settleMeters();
  try {
    const st = await invoke('pause');
    S.t = clampT(st.seconds);
  } catch (e) { /* keep the last known time */ }
  updateTime();
  showFrame(S.t);
}
function toggle() { (S.playing ? pause : play)(); }
async function seek(t) {
  if (!has()) return;
  if (S.playing) await pause();
  // the playhead sits on a frame, so the timecode, the picture and a cut all agree
  S.t = clampT(Number.isFinite(t) ? snapFrame(t) : t);
  invoke('seek', { seconds: S.t }).catch(() => {});
  updateTime();
  showFrame(S.t);
}
function seekBy(d) { seek(S.t + d); scrollToTime(S.t); }
/** Up / Down: the previous or next cut. */
function seekCut(dir) {
  const half = frameStep() / 2, list = cuts();
  const t = dir > 0 ? list.find(x => x > S.t + half) : [...list].reverse().find(x => x < S.t - half);
  if (t !== undefined) { seek(t); scrollToTime(S.t); }
}
/** Keeps the playhead in view while playing, a page at a time. */
function followPlayhead() {
  const sc = scroller(), x = PAD + S.t * S.px;
  if (x < sc.scrollLeft || x > sc.scrollLeft + sc.clientWidth - 40) { sc.scrollLeft = Math.max(0, x - 80); drawVisible(); }
}

/* ---------- export ---------- */
/** Where the video goes: TikTok, YouTube… Resolves to the choice, or null when cancelled. */
function pickExport(choices) {
  const dlg = $('#export-pick'), box = $('#export-choices');
  let want = 'project';
  try { want = localStorage.getItem('clipcut.export') || want; } catch (_) {}
  if (!choices.some(c => c.id === want)) want = choices[0].id;
  box.innerHTML = '';
  const note = () => { const c = choices.find(x => x.id === want); $('#export-note').textContent = (c && c.note) || ''; };
  for (const c of choices) {
    const b = document.createElement('button');
    b.className = 'choice';
    b.id = `export-${c.id}`;
    b.setAttribute('role', 'radio');
    b.innerHTML = '<b></b><span></span>';
    b.firstChild.textContent = c.name;
    b.lastChild.textContent = c.detail;
    b.addEventListener('click', () => { want = c.id; b.focus(); for (const x of box.children) x.setAttribute('aria-checked', String(x === b)); note(); });
    b.addEventListener('dblclick', () => $('#export-go').click());
    b.setAttribute('aria-checked', String(c.id === want));
    box.appendChild(b);
  }
  note();
  return new Promise(done => {
    $('#export-go').onclick = () => { try { localStorage.setItem('clipcut.export', want); } catch (_) {} dlg.close(); done(choices.find(c => c.id === want)); };
    $('#export-pick-cancel').onclick = () => { dlg.close(); done(null); };
    dlg.oncancel = () => done(null);
    dlg.showModal();
    box.querySelector('[aria-checked="true"]').focus();
  });
}
async function exportClip() {
  if (!has()) return;
  if (S.playing) await pause();
  await flushSave();
  let choices;
  try { choices = await invoke('export_choices'); } catch (e) { toast(e, true); return; }
  const choice = await pickExport(choices);
  if (!choice) return;
  let path;
  try {
    path = await invoke('pick_export_path', { name: projectName(), ext: choice.ext });
  } catch (e) { toast(e, true); return; }
  if (!path) return;
  const dlg = $('#export-dlg');
  $('#export-title').textContent = `Exporting: ${choice.name}`;
  $('#export-path').textContent = path;
  $('#export-reveal').onclick = () => invoke('reveal', { path }).catch(e => toast(e, true));
  $('#export-close').onclick = () => dlg.close();
  // the graphics card's encoder first; if it fails partway, once more with ClipCut's own encoder
  for (const hardware of [true, false]) {
    const outcome = await runExport(path, choice, hardware, dlg);
    if (outcome !== 'gpu-failed') break;
    toast("The graphics card's video encoder stopped partway, so ClipCut is exporting again with its own encoder (slower).", true);
  }
  $('#export-cancel').hidden = true;
  $('#export-close').hidden = false;
}
/** One export job, shown in the Export dialog: 'done', 'failed', 'cancelled', or 'gpu-failed'
 *  (the graphics card's encoder failed: worth trying again without it). */
async function runExport(path, choice, hardware, dlg) {
  const before = await invoke('hardware_frames').catch(() => 0);
  let job;
  try {
    job = await invoke('export_as', { path, choice: choice.id, hardware });
  } catch (e) { toast(e, true); dlg.close(); return 'failed'; }
  $('#export-bar').style.width = '0';
  $('#export-meta').textContent = hardware ? 'Starting…' : "Starting again with ClipCut's own encoder…";
  $('#export-meta').className = 'meta';
  $('#export-cancel').hidden = false;
  $('#export-reveal').hidden = true;
  $('#export-close').hidden = true;
  $('#export-cancel').onclick = () => invoke('cancel_export', { job }).catch(() => {});
  if (!dlg.open) dlg.showModal();
  const t0 = performance.now();
  for (;;) {
    await sleep(250);
    let st;
    try { st = await invoke('export_status', { job }); } catch (e) { st = null; }
    if (!st) { $('#export-meta').textContent = 'The export stopped reporting progress.'; $('#export-meta').className = 'meta err'; return 'failed'; }
    $('#export-bar').style.width = (st.progress * 100).toFixed(1) + '%';
    const eta = st.etaSeconds != null ? ` · about ${Math.ceil(st.etaSeconds)} s left` : '';
    $('#export-meta').textContent = `${Math.floor(st.progress * 100)}%${eta}`;
    if (st.finished) {
      if (st.error) {
        const cancelled = /cancel/i.test(st.error);
        // the hardware encoders' errors say how to turn them off
        if (!cancelled && hardware && /Hardware encoding|NVENC|Media Foundation|MFT/i.test(st.error)) return 'gpu-failed';
        $('#export-title').textContent = cancelled ? 'Export cancelled' : 'Export failed';
        $('#export-meta').textContent = cancelled ? 'Nothing was saved.' : cap(st.error);
        $('#export-meta').className = 'meta err';
        return cancelled ? 'cancelled' : 'failed';
      }
      const gpu = (await invoke('hardware_frames').catch(() => before)) > before;
      $('#export-title').textContent = 'Export finished';
      $('#export-bar').style.width = '100%';
      $('#export-meta').textContent = `Saved in ${((performance.now() - t0) / 1000).toFixed(1)} s${gpu ? ', encoded on the graphics card' : ''}.`;
      $('#export-meta').className = 'meta ok';
      $('#export-reveal').hidden = false;
      return 'done';
    }
  }
}

/* ---------- wiring ---------- */
const click = (sel, fn) => $(sel).addEventListener('click', fn);
click('#import-btn', importClips);
click('#empty-import', importClips);
click('#import-audio-btn', importSound);
click('#tab-media', () => showTab('media'));
click('#tab-audio', () => showTab('audio'));
click('#tab-text', () => showTab('text'));
click('#tab-captions', () => showTab('captions'));
click('#tab-transitions', () => showTab('transitions'));
click('#tab-graphics', () => openGraphics());
click('#tab-filters', () => showTab('filters'));
click('#tab-mixer', () => showTab('mixer'));
click('#proxy-make', makeProxies);
$('#proxy-use').addEventListener('change', e => edit('set_proxies', { on: e.target.checked }));
// the player's quality is the viewer's own choice
try { const q = localStorage.getItem('clipcut.quality'); if (q && [...$('#quality').options].some(o => o.value === q)) $('#quality').value = q; } catch (_) {}
$('#quality').addEventListener('change', () => {
  try { localStorage.setItem('clipcut.quality', quality()); } catch (_) {}
  $('#quality').blur();
  if (S.playing) { pause().then(play); } else if (has()) showFrame(S.t);
});
click('#scopes-btn', () => setScopes(!SC.on));
click('#scopes-close', () => setScopes(false));
$('#scope-kind').addEventListener('change', () => { setScopes(true); $('#scope-kind').blur(); });
try { const k = localStorage.getItem('clipcut.scopes'); if (k) { $('#scope-kind').value = k; setScopes(true); } } catch (_) {}
$('#fl-strength').addEventListener('input', e => sendStrength(e.target.value, flKey));
$('#fl-strength').addEventListener('change', e => { flKey = nextKey('strength'); e.target.blur(); });
$('#fl-strength-n').addEventListener('change', e => { if (Number.isFinite(+e.target.value)) { $('#fl-strength').value = e.target.value; sendStrength(+e.target.value, null); } });
click('#graphics-back', closeGraphics);
$('#guide').value = guideKind;
$('#guide').addEventListener('change', e => { guideKind = e.target.value; try { localStorage.setItem('clipcut.guide', guideKind); } catch (_) {} drawOverlay(); e.target.blur(); });
click('#tr-all', transitionEverywhere);
click('#cap-go', autoCaptions);
click('#cap-cancel', () => invoke('cancel_captions'));
click('#cap-fillers', removeFillers);
click('#cap-pauses', removePauses);
click('#cap-srt', saveSubtitles);
$('#cap-on').addEventListener('change', e => edit('set_captions_on', { on: e.target.checked }));
$('#cap-model').addEventListener('change', e => { try { localStorage.setItem('clipcut.model', e.target.value); } catch (_) {} capNote(); });
$('#cap-lang').addEventListener('change', e => { try { localStorage.setItem('clipcut.lang', e.target.value); } catch (_) {} });
listen('caption-progress', e => {
  const p = e.payload || {};
  $('#cap-bar').style.width = (100 * (p.fraction || 0)).toFixed(1) + '%';
  $('#cap-status').textContent = `${Math.floor(100 * (p.fraction || 0))}% · ${p.status || ''}`;
});
wireTranscript();
click('#play-btn', toggle);
click('#export-btn', exportClip);
click('#new-btn', newProject);
click('#open-btn', openProject);
click('#undo-btn', undo);
click('#redo-btn', redo);
click('#split-btn', split);
click('#delleft-btn', () => splitKeep('right'));
click('#delright-btn', () => splitKeep('left'));
click('#delete-btn', deleteSelection);
click('#snap-btn', toggleSnap);
click('#zoomin-btn', () => setPx(S.px * 1.5));
click('#zoomout-btn', () => setPx(S.px / 1.5));
click('#fit-btn', () => fitZoom());
$('#zoom').addEventListener('input', zoomFromSlider);
$('#zoom').addEventListener('change', e => e.target.blur());
$('#ratio').addEventListener('change', e => setRatio(e.target.value));
$('#stage').addEventListener('pointerdown', stagePointerDown);
$('#stage').addEventListener('wheel', stageWheel, { passive: false });
// buttons don't keep the focus, so Space and the editing keys always reach the timeline
document.addEventListener('mousedown', e => { if (e.target.closest('button')) e.preventDefault(); });

$('#ruler').addEventListener('pointerdown', e => scrubFrom(e, false));
content().addEventListener('pointerdown', e => {
  if (e.target.closest('.clip, .ruler')) return;
  scrubFrom(e, true);
});
scroller().addEventListener('scroll', () => {
  $('#heads-inner').style.transform = `translateY(${-scroller().scrollTop}px)`;
  drawVisible();
});
// Zoom: Ctrl + mouse wheel, or a touchpad pinch (the webview sends a pinch as a Ctrl + wheel with
// small, often fractional steps: spreading two fingers is negative deltaY). Anywhere over the
// timeline, the toolbar and the lane heads included, zooming around the pointer.
const pinchZoom = e => {
  if (!(e.ctrlKey || e.metaKey)) return false;
  e.preventDefault();
  const dy = e.deltaMode === 1 ? e.deltaY * 33 : e.deltaY;
  // a pinch moves a few units an event, a mouse wheel notch about 100
  const k = Math.abs(dy) < 50 ? 0.012 : 0.0015;
  const sc = scroller().getBoundingClientRect();
  // a pinch sends dozens of steps a second: one relayout per screen refresh, all steps together
  zoomStep.factor *= Math.exp(-dy * k);
  zoomStep.x = Math.min(sc.right, Math.max(sc.left, e.clientX));
  if (!zoomStep.queued) {
    zoomStep.queued = true;
    requestAnimationFrame(() => {
      zoomStep.queued = false;
      const f = zoomStep.factor;
      zoomStep.factor = 1;
      setPx(S.px * f, zoomStep.x);
    });
  }
  return true;
};
const zoomStep = { factor: 1, x: 0, queued: false };
$('.timeline').addEventListener('wheel', e => { if (!e.target.closest('#tl-scroll')) pinchZoom(e); }, { passive: false });
scroller().addEventListener('wheel', e => {
  if (pinchZoom(e)) {
    // zoomed
  } else if (e.shiftKey || scroller().scrollHeight <= scroller().clientHeight + 1) {
    // a plain wheel scrolls through time; with lanes to spare it scrolls them, Shift through time
    if (Math.abs(e.deltaY) > Math.abs(e.deltaX)) { e.preventDefault(); scroller().scrollLeft += e.deltaY; }
  }
}, { passive: false });

document.addEventListener('keydown', e => {
  if (!$('#graphics-view').hidden) { if (e.key === 'Escape') { e.preventDefault(); closeGraphics(); } return; }
  if (e.target.closest('input, textarea, select') || document.querySelector('dialog[open]') || S.dragging) return;
  // words selected in the transcript: Delete cuts them out of the video
  if (W.sel && !$('#panel-captions').hidden && (e.key === 'Delete' || e.key === 'Backspace')) { e.preventDefault(); if (!e.repeat) cutSelectedWords(); return; }
  const mod = e.ctrlKey || e.metaKey, k = e.key.toLowerCase();
  const key = (cond, fn) => { if (cond) { e.preventDefault(); fn(); return true; } return false; };
  if (mod) {
    key(k === 'z' && !e.shiftKey, undo)
      || key((k === 'z' && e.shiftKey) || k === 'y', redo)
      || key(k === 'a', () => select(visibleClips().map(c => c.id)))
      || key(k === 'i', importClips)
      || key(k === 'e', exportClip)
      || key(k === 'n', newProject)
      || key(k === 'o', openProject)
      || key(k === 's', () => { saveNow(); })
      || key(k === '=' || k === '+', () => setPx(S.px * 1.5))
      || key(k === '-', () => setPx(S.px / 1.5))
      || key(k === '0', () => fitZoom());
    return;
  }
  // , and . : a frame at a time (Shift: 10) — the selected cut moves, Alt slips the clip, else lane clips move
  if ((e.code === 'Comma' || e.code === 'Period') && has()) {
    e.preventDefault();
    const n = (e.code === 'Comma' ? -1 : 1) * (e.shiftKey ? 10 : 1);
    if (e.altKey) slipSel(n); else if (S.cut != null) rollCut(S.cut, n); else nudgeSel(n);
    return;
  }
  if (e.altKey) return;
  // editing keys act once per press, not again while held
  const once = !e.repeat;
  key(e.code === 'Space' || k === 'k', toggle)
    || key(k === 'arrowleft', () => seekBy(e.shiftKey ? -5 : -frameStep()))
    || key(k === 'arrowright', () => seekBy(e.shiftKey ? 5 : frameStep()))
    || key(k === 'j', () => seekBy(-1))
    || key(k === 'l', () => seekBy(1))
    || key(k === 'arrowup', () => seekCut(-1))
    || key(k === 'arrowdown', () => seekCut(1))
    || key(k === 'home', () => { seek(0); scrollToTime(0); })
    || key(k === 'end', () => { seek(Infinity); scrollToTime(dur()); })
    || key(once && k === 's', split)
    || key(once && k === 'q', () => splitKeep('right'))
    || key(once && k === 'w', () => splitKeep('left'))
    || key(once && (k === 'delete' || k === 'backspace'), deleteSelection)
    || key(once && k === 'n', toggleSnap)
    || key(k === 'escape', () => select([]))
    || key(k === '=' || k === '+', () => setPx(S.px * 1.5))
    || key(k === '-', () => setPx(S.px / 1.5));
});
new ResizeObserver(() => { fitCanvas(); if (has() && !S.playing) showFrame(S.t); }).observe($('#stage'));
new ResizeObserver(() => { if (S.tl) layoutTimeline(); }).observe(scroller());

// Files dropped on the window: videos to the end of the main track, sound below it at the playhead.
// While Graphics is open they go to the Clip Studio drop zone under the pointer instead: the window
// takes every drop itself (on Windows the page never sees one), so the files are handed over.
const graphicsOpen = () => !$('#graphics-view').hidden;
listen('tauri://drag-enter', () => { if (!graphicsOpen()) $('#drop-veil').classList.add('on'); });
listen('tauri://drag-leave', () => $('#drop-veil').classList.remove('on'));
listen('tauri://drag-drop', e => {
  $('#drop-veil').classList.remove('on');
  const paths = (e.payload && e.payload.paths) || [];
  if (!paths.length) return;
  if (graphicsOpen()) dropIntoGraphics(paths, e.payload.position);
  else addPaths(paths);
});
const DROP_TYPES = {
  png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', webp: 'image/webp', gif: 'image/gif', avif: 'image/avif',
  mp4: 'video/mp4', m4v: 'video/mp4', mov: 'video/quicktime', webm: 'video/webm', mkv: 'video/x-matroska',
  wav: 'audio/wav', mp3: 'audio/mpeg', m4a: 'audio/mp4', aac: 'audio/aac', flac: 'audio/flac', ogg: 'audio/ogg', opus: 'audio/ogg',
  pdf: 'application/pdf', srt: 'application/x-subrip', vtt: 'text/vtt', txt: 'text/plain',
};
async function dropIntoGraphics(paths, pos) {
  const frame = $('#graphics-frame'), cs = graphicsWin().CS;
  if (!cs || !cs.hosted || !cs.hosted.drop) return;
  const files = [];
  try {
    for (let i = 0; i < paths.length; i++) {
      const name = paths[i].split(/[\\/]/).pop();
      const bytes = await invoke('dropped_file', { index: i });
      files.push(new (graphicsWin().File)([bytes], name, { type: DROP_TYPES[(name.split('.').pop() || '').toLowerCase()] || '' }));
    }
  } catch (err) { toast(String(err), true); return; }
  // the drop's position is in the screen's pixels, the page's in CSS pixels
  const r = frame.getBoundingClientRect(), k = window.devicePixelRatio || 1;
  const x = (pos ? pos.x / k : r.width / 2 + r.left) - r.left, y = (pos ? pos.y / k : r.height / 2 + r.top) - r.top;
  if (!cs.hosted.drop(files, x, y)) toast('Drop it on the preview or a photo, video or sound box of the tool on screen.');
}

renderTextStyles();
// The last project comes back (or files given on the command line start a new one)
invoke('restore').then(r => {
  apply(r.timeline, { fit: true });
  if (r.errors.length) toast(r.errors.join('\n'), true);
}).catch(e => {
  toast(e, true);
  invoke('timeline').then(tl => apply(tl, { fit: true })).catch(() => {});
});

/* ---------- updates ---------- */
// The app asks GitHub Releases for a newer version once at start; a build that can't install
// updates itself (no signing key) only links to the Releases page.
const Upd = { info: null, busy: false, test: false };
function showUpdate() {
  const i = Upd.info;
  $('#about-version').textContent = i ? `Version ${i.current}` : '';
  $('#update-btn').hidden = !(i && i.available && i.can_update);
  if (i && i.available) $('#update-btn').textContent = `Update to ${i.available}`;
  $('#about-install').hidden = !(i && i.available && i.can_update) || Upd.busy;
  $('#about-check').hidden = !(i && i.can_update) || Upd.busy;
  $('#about-notes').hidden = !(i && i.available && i.notes);
  // release notes are GitHub's Markdown: shown as plain text
  $('#about-notes').textContent = ((i && i.notes) || '').replace(/^#+\s*/gm, '').replace(/\*\*/g, '');
}
async function checkUpdate(quiet) {
  if (Upd.busy) return;
  if (!quiet) $('#about-status').textContent = 'Checking for updates…';
  try {
    Upd.info = await invoke('check_update');
    const i = Upd.info;
    $('#about-status').textContent = !i.can_update
      ? 'This copy of ClipCut doesn\'t update itself. New versions are on the Releases page.'
      : i.available ? `ClipCut ${i.available} is ready to install. Your project is saved first, then ClipCut restarts.`
      : 'ClipCut is up to date.';
  } catch (e) {
    if (!quiet) $('#about-status').textContent = String(e);
  }
  showUpdate();
  // the end-to-end update test (Windows CI) presses Update and restart itself
  if (Upd.test && Upd.info && Upd.info.available && Upd.info.can_update) installUpdate();
}
async function installUpdate() {
  if (Upd.busy) return;
  if (S.playing) await pause();
  await flushSave();
  Upd.busy = true;
  showUpdate();
  $('#about-status').textContent = `Downloading ClipCut ${Upd.info.available}…`;
  if (!$('#about-dlg').open) $('#about-dlg').showModal();
  try {
    await invoke('install_update');
  } catch (e) {
    $('#about-status').textContent = String(e);
  }
  Upd.busy = false;
  showUpdate();
}
$('#about-btn').onclick = () => {
  showUpdate();
  $('#about-dlg').showModal();
  if (!Upd.info) checkUpdate(false);
};
$('#about-check').onclick = () => checkUpdate(false);
$('#about-install').onclick = installUpdate;
$('#update-btn').onclick = installUpdate;
$('#about-releases').onclick = () => invoke('open_releases').catch(e => toast(e, true));
$('#about-close').onclick = () => $('#about-dlg').close();
$('#about-dlg').oncancel = e => { if (Upd.busy) e.preventDefault(); };
invoke('update_test').then(t => { Upd.test = !!t; }).catch(() => {});
setTimeout(() => checkUpdate(true), 3000);

// For tests and debugging in the web inspector
window.__clipcut = { S, apply, openGraphics, closeGraphics, selectCut, pickTransition, mainCuts, transitionEverywhere, seek, play, pause, split, splitKeep, deleteSelection, undo, redo, select, setPx, fitZoom, saveNow, addPaths, mainClips, addToLane, addText, queueProps, showTab, queueMixer, takeLevels, slipSel, nudgeSel, rollCut, setDucking, previewAnimation, pickExport, addTitle, saveAsTitle, renderTitles, remakeGraphics, offerRemake, checkUpdate, Upd };
