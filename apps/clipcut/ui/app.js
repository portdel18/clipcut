/* ClipCut interface: talks to the Rust side through Tauri commands (apps/clipcut/src/main.rs and
   editing.rs). Every edit comes back as the whole timeline (`TimelineView`), which `apply` redraws.
   The main track is magnetic: its clips always sit back to back, so a delete or a trim closes up.
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

const S = {
  tl: null,              // TimelineView from the engine
  t: 0,                  // playhead, seconds
  playing: false,
  px: 60,                // timeline zoom, px per second
  sel: new Set(),        // selected main-track clip ids
  snap: true,
  frameBusy: false,      // a preview frame request is in flight
  want: null,            // the latest preview asked for while busy
  geom: new Map(),       // clip id → where a drag shows it (overrides its real place)
  dragging: false,
  els: new Map(),        // clip id → element
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
const cap = s => s ? s[0].toUpperCase() + s.slice(1) : s;
const pad = n => String(n).padStart(2, '0');
const sleep = ms => new Promise(r => setTimeout(r, ms));
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
const has = () => !!(S.tl && S.tl.hasSequence);
const dur = () => has() ? S.tl.duration : 0;
const frameStep = () => 1 / Math.max(1, S.tl ? S.tl.fps : 30);
const clampT = t => has() ? Math.min(Math.max(0, t), Math.max(0, dur() - frameStep())) : 0;
const snapFrame = t => Math.round(t / frameStep()) * frameStep();
/** Main-track clips in order. */
const mainClips = () => S.tl ? S.tl.clips.filter(c => c.track === 'V1').sort((a, b) => a.start - b.start) : [];
const hasSound = c => !!c.link && S.tl.clips.some(a => a.link === c.link && a.track.startsWith('A'));
const mediaOf = item => S.tl && S.tl.media.find(m => m.item === item);
const geom = c => S.geom.get(c.id) || c;
const content = () => $('#tl-content');
const scroller = () => $('#tl-scroll');
/** The timeline time under a pointer. */
const timeAt = clientX => (clientX - content().getBoundingClientRect().left - PAD) / S.px;

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

/** Redraws everything from a new timeline. */
function apply(tl, { fit = false } = {}) {
  const had = has();
  S.tl = tl;
  const ids = new Set(tl.clips.map(c => c.id));
  for (const id of [...S.sel]) if (!ids.has(id)) S.sel.delete(id);
  S.geom.clear();
  S.t = clampT(S.t);
  if (fit || (!had && has())) fitZoom(false);
  $('#empty').hidden = has();
  $('#play-btn').disabled = !has();
  $('#export-btn').disabled = !has();
  $('#tc-total').textContent = fmtFrames(dur());
  renderProject();
  renderMedia();
  renderTrack();
  renderSelection();
  layoutTimeline();
  fitCanvas();
  if (has()) showFrame(S.t); else clearPreview();
  if (tl.project.dirty && has()) saveSoon();
}

/* ---------- preview ---------- */
function fitCanvas() {
  const stage = $('#stage'), c = $('#preview');
  if (!has()) { c.style.width = '0'; c.style.height = '0'; return; }
  const aw = stage.clientWidth - 28, ah = stage.clientHeight - 28;
  const k = Math.min(aw / S.tl.width, ah / S.tl.height);
  c.style.width = Math.max(1, Math.floor(S.tl.width * k)) + 'px';
  c.style.height = Math.max(1, Math.floor(S.tl.height * k)) + 'px';
}
function clearPreview() {
  const c = $('#preview');
  c.getContext('2d').clearRect(0, 0, c.width, c.height);
}
/** Shows a program frame (`{t}`) or a media frame (`{item, t}`, while trimming). Requests never
    pile up: while one renders, only the latest wish is kept. */
async function show(req) {
  if (!has()) return;
  if (S.frameBusy) { S.want = req; return; }
  S.frameBusy = true;
  try {
    const c = $('#preview');
    const maxWidth = Math.min(1920, Math.ceil(parseFloat(c.style.width || '640') * (window.devicePixelRatio || 1)));
    const buf = req.item != null
      ? await invoke('thumb', { item: req.item, seconds: Math.max(0, req.t), maxWidth })
      : await invoke('frame', { seconds: req.t, maxWidth });
    const img = toImage(buf);
    if (c.width !== img.width || c.height !== img.height) { c.width = img.width; c.height = img.height; }
    c.getContext('2d').putImageData(img, 0, 0);
  } catch (e) {
    if (req.item == null) toast(e, true);
  } finally {
    S.frameBusy = false;
    if (S.want) { const next = S.want; S.want = null; show(next); }
  }
}
const showFrame = t => show({ t });

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

async function newProject() {
  await flushSave();
  const tl = await run('new_project');
  if (tl) { S.t = 0; S.sel.clear(); Thumbs.clear(); apply(tl, { fit: true }); toast('New project'); }
}
async function openProject() {
  await flushSave();
  let path;
  try { path = await invoke('pick_project'); } catch (e) { toast(e, true); return; }
  if (!path) return;
  const tl = await run('open_project', { path });
  if (tl) { S.t = 0; S.sel.clear(); Thumbs.clear(); apply(tl, { fit: true }); }
}

/* ---------- media ---------- */
async function importClips() {
  let paths;
  try { paths = await invoke('pick_clips'); } catch (e) { toast(e, true); return; }
  if (paths && paths.length) await addPaths(paths);
}
async function addPaths(paths) {
  toast(paths.length > 1 ? `Adding ${paths.length} files…` : 'Adding…');
  const r = await run('add_clips', { paths });
  if (!r) return;
  apply(r.timeline);
  if (r.errors.length) toast(r.errors.join('\n'), true);
  else $('#toast').hidden = true;
}
let mediaSig = '';
const mediaThumbs = new Map();   // item → canvas
function renderMedia() {
  const media = S.tl.media;
  const sig = JSON.stringify(media.map(m => [m.item, m.name, m.duration]));
  if (sig === mediaSig) return;
  mediaSig = sig;
  const list = $('#media-list');
  list.innerHTML = '';
  for (const m of media) {
    const card = document.createElement('div');
    const video = m.kind !== 'audio';
    card.className = 'media-card' + (video ? '' : ' audio');
    card.dataset.item = m.item;
    card.title = m.path;
    card.innerHTML = (video ? '<canvas width="240" height="135"></canvas>'
      : '<div class="sound"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M9 18V6l10-2v12"/><circle cx="6.5" cy="18" r="2.5"/><circle cx="16.5" cy="16" r="2.5"/></svg></div>')
      + '<button class="add"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 5v14M5 12h14"/></svg></button>'
      + '<div class="meta"><span></span><span class="dur"></span></div>';
    card.querySelector('.meta span').textContent = m.name;
    card.querySelector('.dur').textContent = fmtLen(m.duration);
    const add = card.querySelector('.add');
    add.disabled = !video;
    add.title = video ? 'Add to the end of the main track' : 'Sound tracks arrive in the next update';
    add.setAttribute('aria-label', `Add ${m.name}`);
    add.addEventListener('click', e => { e.stopPropagation(); appendMedia(m.item); });
    if (video) {
      card.addEventListener('pointerdown', e => { if (!e.target.closest('.add') && e.button === 0) startMediaDrag(m, e); });
      card.addEventListener('dblclick', () => appendMedia(m.item));
      paintMediaThumb(m, card.querySelector('canvas'));
    }
    list.appendChild(card);
  }
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

/* ---------- timeline layout ---------- */
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

function renderTrack() {
  const track = $('#track');
  track.innerHTML = '';
  S.els.clear();
  const clips = mainClips();
  if (!clips.length) {
    track.innerHTML = '<div class="track-empty">Drop videos here, or press + on a Media card</div>';
    return;
  }
  for (const c of clips) {
    const el = document.createElement('div');
    el.className = 'clip';
    el.dataset.id = c.id;
    el.innerHTML = '<div class="pics"><canvas></canvas></div><div class="snd"></div><span class="label"></span>'
      + '<div class="handle in" title="Drag to trim the start"></div><div class="handle out" title="Drag to trim the end"></div>';
    el.querySelector('.label').textContent = `${c.name} · ${fmtLen(c.end - c.start)}`;
    el.querySelector('.snd').classList.toggle('none', !hasSound(c));
    el.addEventListener('pointerdown', e => clipPointerDown(c, el, e));
    track.appendChild(el);
    S.els.set(c.id, el);
  }
}
let extraWidth = 0;   // room a drag past the end needs
function layoutTimeline() {
  const sc = scroller();
  const width = Math.max(sc.clientWidth, PAD + dur() * S.px + 240 + extraWidth);
  content().style.width = width + 'px';
  layoutClips();
  syncZoomSlider();
  updateTime();
  drawVisible();
}
function layoutClips() {
  for (const c of mainClips()) {
    const el = S.els.get(c.id);
    if (!el) continue;
    const g = geom(c);
    const w = Math.max(2, (g.end - g.start) * S.px);
    el.style.left = (g.start * S.px) + 'px';
    el.style.width = w + 'px';
    el.classList.toggle('narrow', w < 48);
  }
}
let drawQueued = 0;
/** Ruler and filmstrips for what is on screen, once per frame at most. */
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

/* ---------- filmstrips: media pictures, cached by (media, time) so edits reuse them ---------- */
const Thumbs = {
  cache: new Map(),       // "item:time" → canvas, or null when it couldn't be read
  queue: [],
  running: false,
  clear() { this.cache.clear(); this.queue = []; mediaThumbs.clear(); mediaSig = ''; },
};
function drawStrips() {
  const sc = scroller();
  const viewL = sc.scrollLeft - PAD - 200, viewR = sc.scrollLeft - PAD + sc.clientWidth + 200;
  const need = [];
  for (const c of mainClips()) {
    const el = S.els.get(c.id);
    if (el) drawStrip(c, el, viewL, viewR, need);
  }
  Thumbs.queue = [...new Set(need)];
  pumpThumbs();
}
function drawStrip(c, el, viewL, viewR, need) {
  const g = geom(c);
  const cv = el.querySelector('.pics canvas');
  const L = g.start * S.px, R = g.end * S.px;
  const a = Math.max(L, viewL), b = Math.min(R, viewR);
  if (b <= a) { cv.style.display = 'none'; return; }
  const h = el.clientHeight - 14;
  if (h <= 0) return;
  const m = mediaOf(c.item);
  const aspect = m && m.width && m.height ? m.width / m.height : S.tl.width / Math.max(1, S.tl.height);
  const tileW = Math.max(16, h * aspect);
  const dpr = window.devicePixelRatio || 1;
  cv.style.display = '';
  cv.style.left = (a - L) + 'px';
  cv.style.width = (b - a) + 'px';
  const pw = Math.round((b - a) * dpr), ph = Math.round(h * dpr);
  if (cv.width !== pw || cv.height !== ph) { cv.width = pw; cv.height = ph; }
  const ctx = cv.getContext('2d');
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.fillStyle = '#0b0b0d';
  ctx.fillRect(0, 0, b - a, h);
  // a picture per tile, its time quantized to a step about one tile long
  const tileDur = tileW / S.px;
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
    const sc = scroller(), viewL = sc.scrollLeft - PAD - 200, viewR = viewL + sc.clientWidth + 400;
    for (const c of mainClips()) { const el = S.els.get(c.id); if (el) drawStrip(c, el, viewL, viewR, []); }
  }, 60);
}

/* ---------- time, playhead and tools ---------- */
function updateTime() {
  $('#tc-now').textContent = fmtFrames(S.t);
  $('#playhead').style.left = (PAD + S.t * S.px) + 'px';
  renderTools();
}
/** The clips a split at the playhead would cut: the selected ones, or the main track's. */
function cutTargets() {
  const half = frameStep() / 2;
  const pool = S.sel.size ? mainClips().filter(c => S.sel.has(c.id)) : mainClips();
  return pool.filter(c => c.start + half < S.t && c.end - half > S.t);
}
function renderTools() {
  if (!S.tl) return;
  const cut = has() && cutTargets().length > 0;
  for (const id of ['#split-btn', '#delleft-btn', '#delright-btn']) $(id).disabled = !cut;
  $('#delete-btn').disabled = S.sel.size === 0;
  $('#undo-btn').disabled = !S.tl.canUndo;
  $('#redo-btn').disabled = !S.tl.canRedo;
  $('#undo-btn').title = S.tl.undoLabel ? `Undo ${S.tl.undoLabel} (Ctrl+Z)` : 'Undo (Ctrl+Z)';
  $('#redo-btn').title = S.tl.redoLabel ? `Redo ${S.tl.redoLabel} (Ctrl+Shift+Z)` : 'Redo (Ctrl+Shift+Z)';
  $('#snap-btn').setAttribute('aria-pressed', String(S.snap));
}
function setPlayIcon() {
  $('#play-icon').innerHTML = S.playing ? '<path d="M7 5h4v14H7zM13 5h4v14h-4z"/>' : '<path d="M8 5v14l11-7z"/>';
  $('#play-btn').setAttribute('aria-label', S.playing ? 'Pause' : 'Play');
}
function scrollToTime(t) {
  const sc = scroller(), x = PAD + t * S.px;
  if (x < sc.scrollLeft + 40 || x > sc.scrollLeft + sc.clientWidth - 40) sc.scrollLeft = Math.max(0, x - sc.clientWidth / 2);
}

/* ---------- selection and details ---------- */
function select(ids) {
  S.sel = new Set(ids);
  renderSelection();
}
function renderSelection() {
  for (const [id, el] of S.els) el.classList.toggle('sel', S.sel.has(id));
  renderTools();
  renderDetails();
}
function renderDetails() {
  let rows;
  const sel = mainClips().filter(c => S.sel.has(c.id));
  if (!has()) rows = [['Project', projectName()], ['Clips', 'None yet']];
  else if (sel.length === 1) {
    const c = sel[0], m = mediaOf(c.item);
    rows = [
      ['Clip', c.name],
      ['Starts at', fmtFrames(c.start)],
      ['Length', fmtFrames(c.end - c.start)],
      ['From the media', `${tcShort(c.sourceIn)} – ${tcShort(c.sourceOut)}`],
      ['Media length', fmtFrames(c.mediaDuration)],
      ['Sound', hasSound(c) ? 'Its own' : 'None'],
      ['File', m ? m.path : ''],
    ];
  } else if (sel.length > 1) {
    rows = [['Selected', `${sel.length} clips`], ['Total length', fmtFrames(sel.reduce((n, c) => n + c.end - c.start, 0))]];
  } else {
    rows = [
      ['Project', projectName()],
      ['Resolution', `${S.tl.width} × ${S.tl.height}`],
      ['Frame rate', `${+S.tl.fps.toFixed(3)} fps`],
      ['Length', fmtFrames(dur())],
      ['Clips', String(mainClips().length)],
      ['Saved in', S.tl.project.path || 'Not saved yet'],
    ];
  }
  const dl = $('#details');
  dl.innerHTML = rows.map(() => '<div><dt></dt><dd></dd></div>').join('');
  [...dl.children].forEach((d, k) => {
    d.firstChild.textContent = rows[k][0];
    const path = /^(File|Saved in)$/.test(rows[k][0]);
    // a path is cut at its start; the marks keep it reading left to right inside the rtl box
    d.lastChild.textContent = path ? `\u200E${rows[k][1]}\u200E` : rows[k][1];
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
  if (!S.sel.size) { toast('Select a clip to delete'); return; }
  await edit('delete_clips', () => ({ clips: [...S.sel] }));
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
function follow(e, move, end) {
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
    if (!raf) raf = requestAnimationFrame(tick);
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
  if (!S.snap || ev.shiftKey) { hideSnap(); return t; }
  let best = null, reach = SNAP_PX / S.px;
  for (const x of targets) {
    const d = Math.abs(x - t);
    if (d <= reach) { reach = d; best = x; }
  }
  if (best === null) { hideSnap(); return t; }
  const line = $('#snap-line');
  line.style.left = (PAD + best * S.px) + 'px';
  line.hidden = false;
  return best;
}
function hideSnap() { $('#snap-line').hidden = true; }
/** Every cut on the main track, and its ends. */
function cuts() {
  const set = new Set([0]);
  for (const c of mainClips()) { set.add(+c.start.toFixed(6)); set.add(+c.end.toFixed(6)); }
  return [...set].sort((a, b) => a - b);
}

/* Scrubbing: the ruler, or the timeline outside the clips */
function scrubFrom(e, deselect) {
  if (!has() || e.button !== 0) return;
  if (deselect && S.sel.size) select([]);
  const at = ev => seek(snapTo(timeAt(ev.clientX), ev, cuts()));
  at(e);
  follow(e, at, () => {});
}

/* Clips: click selects (Ctrl or Shift adds), drag reorders, the edges trim */
function clipPointerDown(c, el, e) {
  if (e.button !== 0) return;
  e.stopPropagation();
  e.preventDefault();
  const handle = e.target.closest('.handle');
  if (handle) { startTrim(c, handle.classList.contains('in') ? 'in' : 'out', e); return; }
  const add = e.ctrlKey || e.metaKey || e.shiftKey;
  const wasSelected = S.sel.has(c.id);
  if (add) {
    const ids = new Set(S.sel);
    if (wasSelected) ids.delete(c.id); else ids.add(c.id);
    select(ids);
    return;
  }
  if (!wasSelected) select([c.id]);
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
    if (!started) {
      // a plain click on a clip in a group selects just that clip
      if (wasSelected && S.sel.size > 1) select([c.id]);
      return;
    }
    if (!cancelled && to !== from) edit('reorder', { clip: c.id, to });
  });
}

/* Trimming an edge: the clip follows, later clips move with it, the player shows the new edge */
function startTrim(c, edge, e) {
  if (!S.sel.has(c.id)) select([c.id]);
  const f = frameStep(), len = c.end - c.start;
  const lo = edge === 'out' ? -(len - f) : -c.sourceIn;
  const hi = edge === 'out' ? c.mediaDuration - c.sourceOut : len - f;
  const x0 = e.clientX, s0 = scroller().scrollLeft;
  const later = mainClips().filter(o => o.start >= c.end - 1e-6 && o.id !== c.id);
  const base = edge === 'out' ? c.end : c.start;
  // later cuts move with an end trim, so only the playhead and earlier cuts can pull it
  const targets = [S.t, ...(edge === 'out' ? cuts().filter(t => t <= c.start + 1e-6) : cuts())];
  const tip = document.createElement('div');
  tip.className = 'trim-tip';
  document.body.appendChild(tip);
  let d = 0;
  follow(e, ev => {
    const raw = base + (ev.clientX - x0 + scroller().scrollLeft - s0) / S.px;
    d = snapFrame(Math.min(hi, Math.max(lo, snapTo(raw, ev, targets) - base)));
    if (edge === 'out') {
      S.geom.set(c.id, { ...c, end: c.end + d, sourceOut: c.sourceOut + d });
      for (const o of later) S.geom.set(o.id, { ...o, start: o.start + d, end: o.end + d });
      extraWidth = Math.max(0, d * S.px);
    } else {
      S.geom.set(c.id, { ...c, start: c.start + d, sourceIn: c.sourceIn + d });
    }
    layoutTimeline();
    tip.textContent = `${d >= 0 ? '+' : '−'}${fmtLen(Math.abs(d))} · ${fmtLen(len + (edge === 'out' ? d : -d))}`;
    const r = S.els.get(c.id).getBoundingClientRect();
    tip.style.left = (edge === 'out' ? r.right : r.left) + 'px';
    tip.style.top = r.top + 'px';
    show({ item: c.item, t: edge === 'out' ? c.sourceOut + d - f : c.sourceIn + d });
  }, cancelled => {
    tip.remove();
    S.geom.clear();
    extraWidth = 0;
    layoutTimeline();
    if (!cancelled && Math.abs(d) >= f / 2) edit('trim', { clip: c.id, edge, delta: d });
    else showFrame(S.t);
  });
}

/* A Media card dragged onto the main track goes in where it is dropped */
function startMediaDrag(m, e) {
  const x0 = e.clientX, y0 = e.clientY;
  let ghost = null, to = -1, overTimeline = false;
  const clips = mainClips();
  follow(e, ev => {
    if (!ghost) {
      if (Math.hypot(ev.clientX - x0, ev.clientY - y0) < 6) return;
      ghost = document.createElement('div');
      ghost.className = 'media-ghost';
      const src = mediaThumbs.get(m.item);
      if (src) { const c = document.createElement('canvas'); c.width = 120; c.height = 68; drawCover(c.getContext('2d'), src, 0, 0, 120, 68); ghost.appendChild(c); }
      document.body.appendChild(ghost);
    }
    ghost.style.left = ev.clientX + 'px';
    ghost.style.top = ev.clientY + 'px';
    const r = $('.timeline').getBoundingClientRect();
    overTimeline = ev.clientY >= r.top && ev.clientY <= r.bottom && ev.clientX >= r.left && ev.clientX <= r.right;
    S.geom.clear();
    if (!has() || !overTimeline) { to = -1; $('#insert-mark').hidden = true; layoutClips(); drawVisible(); return; }
    const t = timeAt(ev.clientX);
    to = clips.filter(c => (c.start + c.end) / 2 < t).length;
    const at = to < clips.length ? clips[to].start : dur();
    for (const c of clips.slice(to)) S.geom.set(c.id, { ...c, start: c.start + m.duration, end: c.end + m.duration });
    const mark = $('#insert-mark');
    mark.style.left = (PAD + at * S.px) + 'px';
    mark.hidden = false;
    layoutClips();
    drawVisible();
  }, cancelled => {
    ghost?.remove();
    $('#insert-mark').hidden = true;
    S.geom.clear();
    layoutClips();
    drawVisible();
    if (cancelled || !ghost) return;
    if (to >= 0) edit('insert_media', { item: m.item, index: to });
    // nothing on the timeline yet: dropping anywhere on it starts the sequence
    else if (!has() && overTimeline) appendMedia(m.item);
  });
}

/* ---------- playback ---------- */
let raf = 0;
function stopLoop() { cancelAnimationFrame(raf); raf = 0; }
async function play() {
  if (!has()) return;
  if (S.t >= dur() - frameStep()) S.t = 0;
  try {
    await invoke('play', { from: S.t });
  } catch (e) { toast(e, true); return; }
  S.playing = true;
  setPlayIcon();
  const loop = async () => {
    if (!S.playing) return;
    const st = await invoke('playback');
    if (!S.playing) return;
    S.t = st.seconds;
    $('#sound-note').textContent = st.sound ? '' : 'No audio device: playing without sound';
    updateTime();
    followPlayhead();
    showFrame(S.t);
    if (!st.playing) { S.playing = false; setPlayIcon(); return; }
    raf = requestAnimationFrame(loop);
  };
  raf = requestAnimationFrame(loop);
}
async function pause() {
  stopLoop();
  S.playing = false;
  setPlayIcon();
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
async function exportClip() {
  if (!has()) return;
  if (S.playing) await pause();
  await flushSave();
  let path;
  try {
    path = await invoke('pick_export_path', { name: projectName() });
  } catch (e) { toast(e, true); return; }
  if (!path) return;
  let job;
  try {
    job = await invoke('export', { path });
  } catch (e) { toast(e, true); return; }
  const dlg = $('#export-dlg');
  $('#export-title').textContent = 'Exporting';
  $('#export-path').textContent = path;
  $('#export-bar').style.width = '0';
  $('#export-meta').textContent = 'Starting…';
  $('#export-meta').className = 'meta';
  $('#export-cancel').hidden = false;
  $('#export-reveal').hidden = true;
  $('#export-close').hidden = true;
  $('#export-cancel').onclick = () => invoke('cancel_export', { job }).catch(() => {});
  $('#export-reveal').onclick = () => invoke('reveal', { path }).catch(e => toast(e, true));
  $('#export-close').onclick = () => dlg.close();
  dlg.showModal();
  const t0 = performance.now();
  for (;;) {
    await sleep(250);
    let st;
    try { st = await invoke('export_status', { job }); } catch (e) { st = null; }
    if (!st) { $('#export-meta').textContent = 'The export stopped reporting progress.'; $('#export-meta').className = 'meta err'; break; }
    $('#export-bar').style.width = (st.progress * 100).toFixed(1) + '%';
    const eta = st.etaSeconds != null ? ` · about ${Math.ceil(st.etaSeconds)} s left` : '';
    $('#export-meta').textContent = `${Math.floor(st.progress * 100)}%${eta}`;
    if (st.finished) {
      if (st.error) {
        const cancelled = /cancel/i.test(st.error);
        $('#export-title').textContent = cancelled ? 'Export cancelled' : 'Export failed';
        $('#export-meta').textContent = cancelled ? 'Nothing was saved.' : cap(st.error);
        $('#export-meta').className = 'meta err';
      } else {
        $('#export-title').textContent = 'Export finished';
        $('#export-bar').style.width = '100%';
        $('#export-meta').textContent = `Saved in ${((performance.now() - t0) / 1000).toFixed(1)} s.`;
        $('#export-meta').className = 'meta ok';
        $('#export-reveal').hidden = false;
      }
      break;
    }
  }
  $('#export-cancel').hidden = true;
  $('#export-close').hidden = false;
}

/* ---------- wiring ---------- */
const click = (sel, fn) => $(sel).addEventListener('click', fn);
click('#import-btn', importClips);
click('#empty-import', importClips);
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
// buttons don't keep the focus, so Space and the editing keys always reach the timeline
document.addEventListener('mousedown', e => { if (e.target.closest('button')) e.preventDefault(); });

$('#ruler').addEventListener('pointerdown', e => scrubFrom(e, false));
content().addEventListener('pointerdown', e => {
  if (e.target.closest('.clip, .ruler')) return;
  scrubFrom(e, true);
});
scroller().addEventListener('scroll', drawVisible);
scroller().addEventListener('wheel', e => {
  if (e.ctrlKey || e.metaKey) {
    e.preventDefault();
    setPx(S.px * Math.exp(-e.deltaY * 0.0015), e.clientX);
  } else if (Math.abs(e.deltaY) > Math.abs(e.deltaX)) {
    e.preventDefault();
    scroller().scrollLeft += e.deltaY;
  }
}, { passive: false });

document.addEventListener('keydown', e => {
  if (e.target.closest('input, textarea, select') || document.querySelector('dialog[open]') || S.dragging) return;
  const mod = e.ctrlKey || e.metaKey, k = e.key.toLowerCase();
  const key = (cond, fn) => { if (cond) { e.preventDefault(); fn(); return true; } return false; };
  if (mod) {
    key(k === 'z' && !e.shiftKey, undo)
      || key((k === 'z' && e.shiftKey) || k === 'y', redo)
      || key(k === 'a', () => select(mainClips().map(c => c.id)))
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

// Files dropped on the window go to the end of the main track, in order
listen('tauri://drag-enter', () => $('#drop-veil').classList.add('on'));
listen('tauri://drag-leave', () => $('#drop-veil').classList.remove('on'));
listen('tauri://drag-drop', e => {
  $('#drop-veil').classList.remove('on');
  const paths = (e.payload && e.payload.paths) || [];
  if (paths.length) addPaths(paths);
});

// The last project comes back (or a file given on the command line starts a new one)
invoke('restore').then(r => {
  apply(r.timeline, { fit: true });
  if (r.errors.length) toast(r.errors.join('\n'), true);
}).catch(e => {
  toast(e, true);
  invoke('timeline').then(tl => apply(tl, { fit: true })).catch(() => {});
});

// For tests and debugging in the web inspector
window.__clipcut = { S, apply, seek, play, pause, split, splitKeep, deleteSelection, undo, redo, select, setPx, fitZoom, saveNow, addPaths, mainClips };
