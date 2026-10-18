/* ClipCut interface: talks to the Rust side through Tauri commands (see apps/clipcut/src/main.rs).
   The preview shows the frame the engine renders at the playback clock's time; while playing, the
   sound card is the clock and the picture follows it. */
'use strict';
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const $ = s => document.querySelector(s);

const S = {
  info: null,        // SequenceInfo from the engine
  t: 0,              // playhead, seconds
  playing: false,
  pxPerSec: 60,      // timeline zoom
  fitPx: 60,         // zoom that fits the whole clip
  frameBusy: false,  // a preview frame request is in flight
  wantT: null,       // the latest time asked for while busy
  stripGen: 0,       // cancels an old filmstrip when a new clip opens
};

/* ---------- small helpers ---------- */
let toastTimer = 0;
function toast(msg, err = false) {
  const el = $('#toast');
  el.textContent = msg;
  el.classList.toggle('err', err);
  el.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { el.hidden = true; }, 5000);
}
const pad = n => String(n).padStart(2, '0');
function fmt(sec) {
  sec = Math.max(0, sec || 0);
  const h = Math.floor(sec / 3600), m = Math.floor(sec / 60) % 60, s = Math.floor(sec) % 60;
  return `${pad(h)}:${pad(m)}:${pad(s)}`;
}
function fmtFrames(sec, fps) {
  const f = Math.floor((sec % 1) * fps + 1e-6);
  return `${fmt(sec)}:${pad(f)}`;
}
/** Decodes the frame command's reply: width, height (u32 LE), then RGBA. */
function toImage(buf) {
  const dv = new DataView(buf);
  const w = dv.getUint32(0, true), h = dv.getUint32(4, true);
  return new ImageData(new Uint8ClampedArray(buf, 8, w * h * 4), w, h);
}
function frameStep() { return S.info ? 1 / Math.max(1, S.info.fps) : 1 / 30; }
function clampT(t) { return S.info ? Math.min(Math.max(0, t), Math.max(0, S.info.durationSeconds - frameStep())) : 0; }

/* ---------- preview ---------- */
function fitCanvas() {
  const stage = $('#stage'), c = $('#preview');
  if (!S.info) { c.style.width = '0'; c.style.height = '0'; return; }
  const aw = stage.clientWidth - 28, ah = stage.clientHeight - 28;
  const k = Math.min(aw / S.info.width, ah / S.info.height);
  c.style.width = Math.max(1, Math.floor(S.info.width * k)) + 'px';
  c.style.height = Math.max(1, Math.floor(S.info.height * k)) + 'px';
}
/** Shows the frame at `t`. Requests never pile up: while one is rendering, only the latest wish is kept. */
async function showFrame(t) {
  if (!S.info) return;
  if (S.frameBusy) { S.wantT = t; return; }
  S.frameBusy = true;
  try {
    const c = $('#preview');
    const maxW = Math.min(1920, Math.ceil(parseFloat(c.style.width || '640') * (window.devicePixelRatio || 1)));
    const img = toImage(await invoke('frame', { seconds: t, maxWidth: maxW }));
    if (c.width !== img.width || c.height !== img.height) { c.width = img.width; c.height = img.height; }
    c.getContext('2d').putImageData(img, 0, 0);
  } catch (e) {
    toast(String(e), true);
  } finally {
    S.frameBusy = false;
    if (S.wantT !== null) { const next = S.wantT; S.wantT = null; showFrame(next); }
  }
}

/* ---------- time display and timeline ---------- */
function updateTime() {
  $('#tc-now').textContent = S.info ? fmtFrames(S.t, S.info.fps) : '00:00:00';
  $('#playhead').style.left = (16 + S.t * S.pxPerSec) + 'px';
}
function setPlayIcon() {
  $('#play-icon').innerHTML = S.playing ? '<path d="M7 5h4v14H7zM13 5h4v14h-4z"/>' : '<path d="M8 5v14l11-7z"/>';
  $('#play-btn').setAttribute('aria-label', S.playing ? 'Pause' : 'Play');
}
function layoutTimeline() {
  const scroll = $('#tl-scroll');
  const dur = S.info ? S.info.durationSeconds : 0;
  S.fitPx = dur > 0 ? Math.max(4, (scroll.clientWidth - 48) / dur) : 60;
  const z = +$('#zoom').value / 100;                 // 0 = fit, 1 = 400 px per second
  S.pxPerSec = S.fitPx * Math.pow(Math.max(1, 400 / S.fitPx), z);
  const width = dur * S.pxPerSec;
  $('#tl-content').style.width = (width + 48) + 'px';
  const clip = $('#track .clip');
  if (clip) clip.style.width = width + 'px';
  drawRuler(dur, width);
  updateTime();
  layoutStrip();
}
function drawRuler(dur, width) {
  const r = $('#ruler');
  r.innerHTML = '';
  if (!dur) return;
  // A labelled tick every 1, 2, 5, 10, 30 or 60 s, whichever keeps labels about 90 px apart
  const steps = [0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600];
  const major = steps.find(s => s * S.pxPerSec >= 90) || 600;
  const minor = major / 5;
  const frag = document.createDocumentFragment();
  // Count ticks rather than adding up `minor`, which drifts (0.2 × 10 is not exactly 2)
  for (let i = 0; i * minor <= dur + 1e-6; i++) {
    const t = i * minor, isMajor = i % 5 === 0;
    const x = t * S.pxPerSec;
    if (x > width + 1) break;
    const tick = document.createElement('div');
    tick.className = 'tick' + (isMajor ? ' major' : '');
    tick.style.left = x + 'px';
    frag.appendChild(tick);
    if (isMajor) {
      const lab = document.createElement('span');
      lab.className = 'lab';
      lab.style.left = x + 'px';
      lab.textContent = fmt(t).replace(/^00:/, '');
      frag.appendChild(lab);
    }
  }
  r.appendChild(frag);
}
/** Seeks to the time under the pointer, and keeps following while the button is held. */
function scrubFrom(e) {
  if (!S.info) return;
  const content = $('#tl-content');
  const at = ev => {
    const x = ev.clientX - content.getBoundingClientRect().left - 16;
    seek(x / S.pxPerSec);
  };
  at(e);
  const move = ev => at(ev);
  const up = () => { window.removeEventListener('pointermove', move); window.removeEventListener('pointerup', up); };
  window.addEventListener('pointermove', move);
  window.addEventListener('pointerup', up);
}

/* ---------- filmstrip on the clip ---------- */
let stripTiles = [];
function layoutStrip() {
  const clip = $('#track .clip');
  if (!clip || !S.info) return;
  const strip = clip.querySelector('.strip');
  const h = clip.clientHeight - 4;
  const tileW = Math.max(24, Math.round(h * S.info.width / S.info.height));
  const width = S.info.durationSeconds * S.pxPerSec;
  const n = Math.min(400, Math.max(1, Math.ceil(width / tileW)));
  if (n === stripTiles.length && strip.firstChild && strip.firstChild.style.width === tileW + 'px') return;
  strip.innerHTML = '';
  stripTiles = [];
  for (let i = 0; i < n; i++) {
    const c = document.createElement('canvas');
    c.style.width = tileW + 'px';
    strip.appendChild(c);
    stripTiles.push({ c, t: Math.min(S.info.durationSeconds - frameStep(), (i + 0.5) * tileW / S.pxPerSec) });
  }
  fillStrip(++S.stripGen);
}
/** Renders filmstrip tiles one by one, behind the preview (it waits while the preview is busy). */
async function fillStrip(gen) {
  const tiles = stripTiles.slice();
  for (const tile of tiles) {
    if (gen !== S.stripGen) return;
    while (S.frameBusy || S.playing) {
      await new Promise(r => setTimeout(r, 60));
      if (gen !== S.stripGen) return;
    }
    try {
      const img = toImage(await invoke('frame', { seconds: Math.max(0, tile.t), maxWidth: 160 }));
      tile.c.width = img.width; tile.c.height = img.height;
      tile.c.getContext('2d').putImageData(img, 0, 0);
    } catch (e) { return; }
  }
}

/* ---------- open ---------- */
async function openPath(path) {
  if (!path) return;
  stopLoop();
  $('#clip-title').textContent = 'Opening…';
  try {
    const info = await invoke('open_clip', { path });
    S.info = info; S.t = 0; S.playing = false;
    setPlayIcon();
    $('#clip-title').textContent = info.name;
    $('#empty').hidden = true;
    $('#export-btn').disabled = false;
    $('#play-btn').disabled = false;
    $('#tc-total').textContent = fmtFrames(info.durationSeconds, info.fps);
    renderDetails(path);
    renderTrack();
    fitCanvas();
    layoutTimeline();
    await showFrame(0);
    renderMediaCard();
  } catch (e) {
    $('#clip-title').textContent = S.info ? S.info.name : 'No clip open';
    toast(String(e), true);
  }
}
async function importClip() {
  try {
    const path = await invoke('pick_clip');
    if (path) await openPath(path);
  } catch (e) { toast(String(e), true); }
}
function renderDetails(path) {
  const i = S.info;
  const rows = [
    ['Clip', i.name],
    ['Resolution', `${i.width} × ${i.height}`],
    ['Frame rate', `${+i.fps.toFixed(3)} fps`],
    ['Duration', fmtFrames(i.durationSeconds, i.fps)],
    ['Sample rate', `${(i.sampleRate / 1000).toFixed(1)} kHz`],
    ['File', path],
  ];
  $('#details').innerHTML = rows.map(([k, v]) => `<div><dt>${k}</dt><dd></dd></div>`).join('');
  [...$('#details').querySelectorAll('dd')].forEach((dd, k) => { dd.textContent = rows[k][1]; });
}
function renderTrack() {
  const track = $('#track');
  track.innerHTML = '<div class="clip"><div class="strip"></div><span class="label"></span></div>';
  track.querySelector('.label').textContent = `${S.info.name} · ${fmt(S.info.durationSeconds)}`;
  stripTiles = [];
}
async function renderMediaCard() {
  const list = $('#media-list');
  list.innerHTML = '<div class="media-card"><canvas></canvas><div class="meta"><span></span><span class="dur"></span></div></div>';
  list.querySelector('.meta span').textContent = S.info.name;
  list.querySelector('.dur').textContent = fmt(S.info.durationSeconds);
  try {
    const img = toImage(await invoke('frame', { seconds: Math.min(1, S.info.durationSeconds / 2), maxWidth: 240 }));
    const c = list.querySelector('canvas');
    c.width = img.width; c.height = img.height;
    c.getContext('2d').putImageData(img, 0, 0);
  } catch (e) { /* the card keeps its black thumbnail */ }
}

/* ---------- playback ---------- */
let raf = 0;
function stopLoop() { cancelAnimationFrame(raf); raf = 0; }
async function play() {
  if (!S.info) return;
  if (S.t >= S.info.durationSeconds - frameStep()) S.t = 0;
  try {
    await invoke('play', { from: S.t });
  } catch (e) { toast(String(e), true); return; }
  S.playing = true;
  setPlayIcon();
  const loop = async () => {
    if (!S.playing) return;
    const st = await invoke('playback');
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
    S.t = st.seconds;
  } catch (e) { /* keep the last known time */ }
  updateTime();
  showFrame(S.t);
}
function toggle() { (S.playing ? pause : play)(); }
async function seek(t) {
  if (!S.info) return;
  if (S.playing) await pause();
  S.t = clampT(t);
  invoke('seek', { seconds: S.t }).catch(() => {});
  updateTime();
  showFrame(S.t);
}
/** Keeps the playhead in view while playing. */
function followPlayhead() {
  const scroll = $('#tl-scroll');
  const x = 16 + S.t * S.pxPerSec;
  if (x < scroll.scrollLeft || x > scroll.scrollLeft + scroll.clientWidth - 40) scroll.scrollLeft = Math.max(0, x - 80);
}

/* ---------- export ---------- */
async function exportClip() {
  if (!S.info) return;
  if (S.playing) await pause();
  let path;
  try {
    path = await invoke('pick_export_path', { name: S.info.name });
  } catch (e) { toast(String(e), true); return; }
  if (!path) return;
  let job;
  try {
    job = await invoke('export', { path });
  } catch (e) { toast(String(e), true); return; }
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
  $('#export-reveal').onclick = () => invoke('reveal', { path }).catch(e => toast(String(e), true));
  $('#export-close').onclick = () => dlg.close();
  dlg.showModal();
  const t0 = performance.now();
  for (;;) {
    await new Promise(r => setTimeout(r, 250));
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
        $('#export-meta').textContent = cancelled ? 'Nothing was saved.' : st.error;
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
$('#import-btn').addEventListener('click', importClip);
$('#empty-import').addEventListener('click', importClip);
$('#play-btn').addEventListener('click', toggle);
$('#export-btn').addEventListener('click', exportClip);
$('#ruler').addEventListener('pointerdown', scrubFrom);
$('#track').addEventListener('pointerdown', scrubFrom);
$('#zoom').addEventListener('input', layoutTimeline);
document.addEventListener('keydown', e => {
  if (e.target.closest('input, textarea, select') || document.querySelector('dialog[open]')) return;
  if (e.code === 'Space') { e.preventDefault(); toggle(); }
  else if (e.key === 'ArrowLeft') { e.preventDefault(); seek(S.t - (e.shiftKey ? 1 : frameStep())); }
  else if (e.key === 'ArrowRight') { e.preventDefault(); seek(S.t + (e.shiftKey ? 1 : frameStep())); }
  else if (e.key === 'Home') { e.preventDefault(); seek(0); }
  else if (e.key === 'End') { e.preventDefault(); seek(Infinity); }
  else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'i') { e.preventDefault(); importClip(); }
  else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'e') { e.preventDefault(); exportClip(); }
});
new ResizeObserver(() => { fitCanvas(); layoutTimeline(); if (!S.playing) showFrame(S.t); }).observe($('#stage'));

// Files dropped on the window
listen('tauri://drag-enter', () => $('#drop-veil').classList.add('on'));
listen('tauri://drag-leave', () => $('#drop-veil').classList.remove('on'));
listen('tauri://drag-drop', e => {
  $('#drop-veil').classList.remove('on');
  const p = e.payload && e.payload.paths && e.payload.paths[0];
  if (p) openPath(p);
});

// A file given on the command line (or "Open with ClipCut")
invoke('startup_file').then(p => { if (p) openPath(p); }).catch(() => {});

// For tests and debugging in the web inspector
window.__clipcut = { S, openPath, seek, play, pause };
