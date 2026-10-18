/* Clip Studio · core: helpers, controls, presets, player, video export
   Classic script (not an ES module) so index.html still works when opened straight from disk.
   Shared helpers live on window.CS; each tool registers itself in CS.tools. */
(() => {
'use strict';
/* ============================================================
   Shared helpers
   ============================================================ */
const $ = (s, r = document) => r.querySelector(s);
const $$ = (s, r = document) => Array.from(r.querySelectorAll(s));
const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
const lerp = (a, b, t) => a + (b - a) * t;
const smoothstep = t => { t = clamp(t, 0, 1); return t * t * (3 - 2 * t); };
const sleep = ms => new Promise(r => setTimeout(r, ms));
const EASE = {
  smooth: t => (t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2),
  snappy: t => 1 - Math.pow(1 - t, 4),
  linear: t => t,
  glide: t => 1 - Math.pow(1 - t, 3),
};
const ease = (name, t) => (EASE[name] || EASE.smooth)(clamp(t, 0, 1));
function easeOutBounce(x) {
  const n1 = 7.5625, d1 = 2.75;
  if (x < 1 / d1) return n1 * x * x;
  if (x < 2 / d1) return n1 * (x -= 1.5 / d1) * x + 0.75;
  if (x < 2.5 / d1) return n1 * (x -= 2.25 / d1) * x + 0.9375;
  return n1 * (x -= 2.625 / d1) * x + 0.984375;
}
function easeOutBack(t) { const c1 = 1.70158, c3 = c1 + 1; return 1 + c3 * Math.pow(t - 1, 3) + c1 * Math.pow(t - 1, 2); }
function mulberry32(a) { return function () { a |= 0; a = a + 0x6D2B79F5 | 0; let t = Math.imul(a ^ a >>> 15, 1 | a); t = t + Math.imul(t ^ t >>> 7, 61 | t) ^ t; return ((t ^ t >>> 14) >>> 0) / 4294967296; }; }
const pad2 = n => String(n).padStart(2, '0');
function fmtTC(t, fps) {
  const f = Math.max(0, Math.floor(t * fps + 1e-6));
  const s = Math.floor(f / fps);
  return `${pad2(Math.floor(s / 3600))}:${pad2(Math.floor(s / 60) % 60)}:${pad2(s % 60)}:${pad2(f % fps)}`;
}
const fmtSec = v => `${+(+v).toFixed(2)}s`;
const isHex = v => /^#[0-9a-f]{6}$/i.test(v);
const slug = s => (s || 'clip').toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '').slice(0, 40) || 'clip';
const clone = v => JSON.parse(JSON.stringify(v));

/* ---------- inside another app ----------
   An app that embeds Clip Studio (ClipCut, the video editor) sets window.__clipcutHost on this page or on
   the page holding it in a frame: { aspect, begin({name, width, height, fps}) → job, frame(job, i, bytes, ext),
   audio(job, wavBytes), finish(job, info), abort(job) }. Each output panel then gets "Add to video", which
   renders through the same draw function and hands the frames to the app instead of downloading them.
   The app may also hand over its video's frame (CS.hosted.setBackdrop) to preview graphics over it. */
const HOST = (() => {
  if (window.__clipcutHost) return window.__clipcutHost;
  try { return window.parent !== window ? window.parent.__clipcutHost || null : null; } catch (_) { return null; }
})();
if (HOST) document.documentElement.classList.add('hosted');
/* Where a library comes from: the CDN, unless the page names a local copy in window.CLIP_STUDIO_LIBS
   (an app that bundles Clip Studio and allows no outside scripts). */
const lib = (name, cdn) => (window.CLIP_STUDIO_LIBS && window.CLIP_STUDIO_LIBS[name]) || cdn;

const store = {
  get(k, d) { try { const v = localStorage.getItem('clipstudio.' + k); return v == null ? d : JSON.parse(v); } catch (e) { return d; } },
  set(k, v) { try { localStorage.setItem('clipstudio.' + k, JSON.stringify(v)); } catch (e) { /* storage blocked or full */ } },
};

let toastTimer = 0;
function toast(msg, ms = 4200) {
  const el = $('#toast');
  el.textContent = msg; el.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { el.hidden = true; }, ms);
}

function roundRect(ctx, x, y, w, h, r) {
  r = Math.max(0, Math.min(r, w / 2, h / 2));
  ctx.beginPath();
  if (ctx.roundRect) { ctx.roundRect(x, y, w, h, r); return; }
  ctx.moveTo(x + r, y); ctx.arcTo(x + w, y, x + w, y + h, r); ctx.arcTo(x + w, y + h, x, y + h, r);
  ctx.arcTo(x, y + h, x, y, r); ctx.arcTo(x, y, x + w, y, r); ctx.closePath();
}
function loadImageFile(file) {
  return new Promise((res, rej) => {
    const url = URL.createObjectURL(file);
    const img = new Image();
    img.onload = () => res(img);
    img.onerror = () => { URL.revokeObjectURL(url); rej(new Error('That file isn’t an image this browser can open.')); };
    img.src = url;
  });
}

const FRAMES = { '16:9': [1920, 1080], '9:16': [1080, 1920], '1:1': [1080, 1080], '4:5': [1080, 1350] };
function dims(out) {
  const [W, H] = FRAMES[out.aspect] || FRAMES['16:9'];
  const q = out.quality === '4k' ? 2 : 1;
  return { W, H, q, OW: W * q, OH: H * q };
}

const SWATCHES = ['#FF3B30', '#FFD60A', '#FF9F0A', '#34C759', '#0A84FF', '#BF5AF2', '#FFFFFF', '#111111'];

/* ---------- binding layer: each control registers how to re-show its value (used by presets) ---------- */
const registry = new Map();
function reg(obj, fn) { if (!registry.has(obj)) registry.set(obj, []); registry.get(obj).push(fn); }
function resync(obj) { (registry.get(obj) || []).forEach(f => f()); }
const saveTimers = {};
function saveSoon(key, obj) {
  clearTimeout(saveTimers[key]);
  saveTimers[key] = setTimeout(() => store.set(key, obj), 300);
}
function bindRange(id, obj, key, fmt, onChange) {
  const el = $('#' + id), out = $(`output[for="${id}"]`);
  const show = () => { el.value = obj[key]; if (out) out.textContent = fmt ? fmt(obj[key]) : obj[key]; };
  show(); reg(obj, show);
  el.addEventListener('input', () => { obj[key] = parseFloat(el.value); if (out) out.textContent = fmt ? fmt(obj[key]) : obj[key]; onChange(key); });
  return { set(v) { obj[key] = v; show(); } };
}
function bindColor(id, obj, key, onChange) {
  const el = $('#' + id), hex = $('#' + id + '-hex');
  const sw = $(`.swatches[data-target="${id}"]`);
  const syncSw = () => { if (sw) $$('.sw', sw).forEach(b => b.setAttribute('aria-pressed', String(b.dataset.c.toLowerCase() === String(obj[key]).toLowerCase()))); };
  const show = () => { el.value = String(obj[key]).toLowerCase(); if (hex && document.activeElement !== hex) hex.value = String(obj[key]).toUpperCase(); syncSw(); };
  if (sw) {
    sw.innerHTML = SWATCHES.map(c => `<button class="sw" type="button" style="--c:${c}" data-c="${c}" aria-label="Use ${c}" aria-pressed="false"></button>`).join('');
    sw.addEventListener('click', e => { const b = e.target.closest('.sw'); if (!b) return; obj[key] = b.dataset.c; show(); onChange(key); });
  }
  el.addEventListener('input', () => { obj[key] = el.value; show(); onChange(key); });
  if (hex) {
    hex.addEventListener('input', () => { let v = hex.value.trim(); if (!v.startsWith('#')) v = '#' + v; if (isHex(v)) { obj[key] = v; el.value = v.toLowerCase(); syncSw(); onChange(key); } });
    hex.addEventListener('blur', show);
  }
  show(); reg(obj, show);
  return { set(v) { obj[key] = v; show(); } };
}
function bindCheck(id, obj, key, onChange) {
  const el = $('#' + id);
  const show = () => { el.checked = !!obj[key]; };
  show(); reg(obj, show);
  el.addEventListener('change', () => { obj[key] = el.checked; onChange(key); });
  return { set(v) { obj[key] = v; show(); } };
}
function bindSeg(name, obj, key, onChange) {
  const radios = $$(`input[name="${name}"]`);
  const show = () => radios.forEach(r => { r.checked = r.value === String(obj[key]); });
  radios.forEach(r => r.addEventListener('change', () => { if (r.checked) { obj[key] = r.value; onChange(key); } }));
  show(); reg(obj, show);
  return { set(v) { obj[key] = v; show(); } };
}
function bindText(id, obj, key, onChange) {
  const el = $('#' + id);
  const show = () => { if (document.activeElement !== el) el.value = obj[key] || ''; };
  show(); reg(obj, show);
  el.addEventListener('input', () => { obj[key] = el.value; onChange(key); });
  return { set(v) { obj[key] = v; el.value = v; } };
}
function bindSelect(id, obj, key, onChange) {
  const el = $('#' + id);
  const show = () => { el.value = String(obj[key]); };
  show(); reg(obj, show);
  el.addEventListener('change', () => { obj[key] = isNaN(+el.value) || el.value === '' ? el.value : +el.value; onChange(key); });
  return { set(v) { obj[key] = v; show(); } };
}

/* ---------- output panel (shared markup) ---------- */
const ASPECT_LABELS = { '16:9': '16:9 YouTube', '9:16': '9:16 Reels/TikTok', '1:1': '1:1', '4:5': '4:5' };
const outputs = {};
function buildOutputPanel(prefix, out, onChange, actions, { aspects = ['16:9', '9:16', '1:1', '4:5'], both = true } = {}) {
  const host = $(`[data-output="${prefix}"]`);
  outputs[prefix] = { out, onChange, actions, aspects };
  // inside an app, the frame starts the shape of its video
  if (HOST && aspects.includes(HOST.aspect)) out.aspect = HOST.aspect;
  host.innerHTML = `
    <div class="field"><span class="lbl">Frame</span>
      <div class="seg" role="radiogroup" aria-label="Frame shape">
        ${aspects.map(a => `<label><input type="radio" name="${prefix}-aspect" value="${a}"><span>${ASPECT_LABELS[a]}</span></label>`).join('')}
      </div></div>
    <div class="row2">
      <div class="field"><span class="lbl">Resolution</span>
        <div class="seg" role="radiogroup" aria-label="Resolution">
          <label><input type="radio" name="${prefix}-quality" value="1080"><span>HD</span></label>
          <label><input type="radio" name="${prefix}-quality" value="4k"><span>4K</span></label>
        </div></div>
      <div class="field"><span class="lbl">Frame rate</span>
        <div class="seg" role="radiogroup" aria-label="Frame rate">
          <label><input type="radio" name="${prefix}-fps" value="24"><span>24</span></label>
          <label><input type="radio" name="${prefix}-fps" value="30"><span>30</span></label>
          <label><input type="radio" name="${prefix}-fps" value="60"><span>60</span></label>
        </div></div>
    </div>
    <div class="field"><span class="lbl">File</span>
      <div class="seg" role="radiogroup" aria-label="File type">
        <label><input type="radio" name="${prefix}-format" value="mp4"><span>MP4 video</span></label>
        <label><input type="radio" name="${prefix}-format" value="png-seq"><span>PNG frames (.zip)</span></label>
      </div></div>
    ${HOST ? `<div class="btn-row"><button class="btn add" id="${prefix}-add">Add to video</button></div>` : ''}
    <div class="btn-row">
      <button class="btn rec" id="${prefix}-render">Render</button>
      ${both ? `<button class="btn" id="${prefix}-render-both">Render 16:9 + 9:16</button>` : ''}
    </div>
    <div class="btn-row"><button class="btn" id="${prefix}-still">Save this frame (PNG)</button></div>
    <p class="hint" id="${prefix}-outnote"></p>`;
  const wrap = key => () => { if (key === 'fps') out.fps = +out.fps; onChange('out.' + key); };
  bindSeg(`${prefix}-aspect`, out, 'aspect', wrap('aspect'));
  bindSeg(`${prefix}-quality`, out, 'quality', wrap('quality'));
  bindSeg(`${prefix}-fps`, out, 'fps', wrap('fps'));
  bindSeg(`${prefix}-format`, out, 'format', wrap('format'));
  $(`#${prefix}-render`).addEventListener('click', () => actions.render());
  if (HOST) $(`#${prefix}-add`).addEventListener('click', () => addToVideo(prefix));
  if (both) $(`#${prefix}-render-both`).addEventListener('click', () => actions.renderBoth());
  $(`#${prefix}-still`).addEventListener('click', () => actions.still());
}

/* ---------- presets ---------- */
const PRESET_TOOLS = ['ul', 'mp', 'sp', 'sz', 'tl', 'nm', 'au', 'lt', 'th'];
const userPresets = store.get('presets', {});
for (const k of PRESET_TOOLS) if (!userPresets[k] || typeof userPresets[k] !== 'object') userPresets[k] = {};
const presetUI = {};
const restorers = {};   // prefix → { state, afterApply }: what a preset (or the host app) re-applies
function buildPresetPanel(prefix, state, keys, builtins, afterApply) {
  const host = $(`[data-presets="${prefix}"]`);
  host.innerHTML = `
    <div class="field"><label class="lbl" for="${prefix}-preset">Apply a preset</label>
      <select id="${prefix}-preset"></select></div>
    <div class="field"><label class="lbl" for="${prefix}-preset-name">Save the current look as</label>
      <div class="row-inline"><input type="text" id="${prefix}-preset-name" placeholder="e.g. My channel style" maxlength="40"><button class="btn" id="${prefix}-preset-save">Save</button></div></div>
    <div class="btn-row">
      <button class="btn small" id="${prefix}-preset-del" disabled>Delete selected</button>
      <button class="btn small" id="${prefix}-preset-export">Export</button>
      <button class="btn small" id="${prefix}-preset-import">Import</button>
      <input type="file" id="${prefix}-preset-file" accept=".json,application/json" hidden>
    </div>
    <p class="hint">Presets keep colors, fonts, timing, camera, layout, and output settings, not your text, media, or locations. They're saved in this browser; Export moves them to another computer.</p>`;
  const sel = $(`#${prefix}-preset`), del = $(`#${prefix}-preset-del`);
  const fill = (selected = '') => {
    const esc = s => s.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;');
    const saved = Object.keys(userPresets[prefix]).sort((a, b) => a.localeCompare(b));
    sel.innerHTML = `<option value="">Choose…</option><optgroup label="Built in">${Object.keys(builtins).map(n => `<option value="b:${esc(n)}">${esc(n)}</option>`).join('')}</optgroup>` +
      (saved.length ? `<optgroup label="Saved">${saved.map(n => `<option value="u:${esc(n)}">${esc(n)}</option>`).join('')}</optgroup>` : '');
    sel.value = selected;
    del.disabled = !sel.value.startsWith('u:');
  };
  fill();
  presetUI[prefix] = { fill };
  restorers[prefix] = { state, afterApply };
  sel.addEventListener('change', () => {
    const v = sel.value; del.disabled = !v.startsWith('u:');
    if (!v) return;
    const name = v.slice(2);
    const p = v.startsWith('b:') ? builtins[name] : userPresets[prefix][name];
    if (!p) return;
    for (const k of keys) {
      if (!(k in p)) continue;
      if (k === 'out') Object.assign(state.out, clone(p.out));
      else state[k] = clone(p[k]);
    }
    resync(state); resync(state.out);
    afterApply();
    toast(`Applied “${name}”.`);
  });
  $(`#${prefix}-preset-save`).addEventListener('click', () => {
    const inp = $(`#${prefix}-preset-name`);
    const name = inp.value.trim();
    if (!name) { toast('Type a name for the preset first.'); inp.focus(); return; }
    const p = {};
    for (const k of keys) if (k in state) p[k] = clone(state[k]);
    userPresets[prefix][name] = p;
    store.set('presets', userPresets);
    inp.value = ''; fill('u:' + name);
    toast(`Saved preset “${name}”.`);
  });
  del.addEventListener('click', () => {
    if (!sel.value.startsWith('u:')) return;
    const name = sel.value.slice(2);
    delete userPresets[prefix][name];
    store.set('presets', userPresets);
    fill(); toast(`Deleted “${name}”.`);
  });
  $(`#${prefix}-preset-export`).addEventListener('click', () => {
    const n = PRESET_TOOLS.reduce((sum, k) => sum + Object.keys(userPresets[k]).length, 0);
    if (!n) { toast('No saved presets yet. Save one first.'); return; }
    downloadBlob(new Blob([JSON.stringify({ app: 'clip-studio', version: 1, ...userPresets }, null, 2)], { type: 'application/json' }), 'clip-studio-presets.json');
    toast(`Exported ${n} preset${n === 1 ? '' : 's'}.`);
  });
  $(`#${prefix}-preset-import`).addEventListener('click', () => $(`#${prefix}-preset-file`).click());
  $(`#${prefix}-preset-file`).addEventListener('change', async e => {
    const f = e.target.files[0]; e.target.value = '';
    if (!f) return;
    try {
      const data = JSON.parse(await f.text());
      let n = 0;
      for (const p of PRESET_TOOLS) {
        if (data[p] && typeof data[p] === 'object') for (const [name, v] of Object.entries(data[p])) { if (v && typeof v === 'object') { userPresets[p][name.slice(0, 40)] = v; n++; } }
      }
      if (!n) throw new Error();
      store.set('presets', userPresets);
      PRESET_TOOLS.forEach(k => presetUI[k]?.fill());
      toast(`Imported ${n} preset${n === 1 ? '' : 's'}.`);
    } catch (err) { toast('That file isn’t a Clip Studio presets file.'); }
  });
}

/* ---------- transport / player ---------- */
const ICON_PLAY = '<svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true"><path d="M4 2.5v11l9-5.5z" fill="currentColor"/></svg>';
const ICON_PAUSE = '<svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true"><path d="M4 2.5h3v11H4zM9 2.5h3v11H9z" fill="currentColor"/></svg>';
const ICON_START = '<svg width="14" height="14" viewBox="0 0 16 16" aria-hidden="true"><path d="M3 2.5h2v11H3zM14 2.5v11L6 8z" fill="currentColor"/></svg>';

function makePlayer(prefix, opts) {
  const host = $(`[data-player="${prefix}"]`);
  host.innerHTML = `
    <button class="play" id="${prefix}-p-play" aria-label="Play">${ICON_PLAY}</button>
    <button class="iconbtn" id="${prefix}-p-rew" aria-label="Back to start">${ICON_START}</button>
    <input class="scrub" type="range" id="${prefix}-p-scrub" min="0" max="1" step="any" value="0" aria-label="Timeline position">
    <span class="tc"><span id="${prefix}-p-tc">00:00:00:00</span> <span class="tot">/ <span id="${prefix}-p-dur">00:00:00:00</span></span></span>
    <span class="toggles">
      <label class="check"><input type="checkbox" id="${prefix}-p-loop"> Loop</label>
      <select class="overlay-pick" id="${prefix}-p-overlay" aria-label="Overlay on the preview">${OVERLAY_OPTIONS}</select>
    </span>`;
  const els = { play: $(`#${prefix}-p-play`), rew: $(`#${prefix}-p-rew`), scrub: $(`#${prefix}-p-scrub`), tc: $(`#${prefix}-p-tc`), dur: $(`#${prefix}-p-dur`), loop: $(`#${prefix}-p-loop`), overlay: $(`#${prefix}-p-overlay`) };
  const p = { t: 0, playing: false, raf: 0, t0: 0 };
  function sync() {
    const T = opts.total(), fps = opts.fps();
    els.scrub.max = T; els.scrub.value = p.t;
    els.tc.textContent = fmtTC(p.t, fps); els.dur.textContent = fmtTC(T, fps);
    els.play.innerHTML = p.playing ? ICON_PAUSE : ICON_PLAY;
    els.play.setAttribute('aria-label', p.playing ? 'Pause' : 'Play');
    if (opts.track) { const head = opts.track.querySelector('.head'); if (head) head.style.left = (T ? (p.t / T) * 100 : 0) + '%'; }
  }
  function tick(now) {
    if (!p.playing) return;
    const T = opts.total();
    let t = (now - p.t0) / 1000;
    if (t >= T) {
      if (els.loop.checked) { p.t0 = now; t = 0; opts.onLoop?.(); } else { t = T; p.playing = false; }
    }
    p.t = t; opts.render(t); sync();
    if (p.playing) p.raf = requestAnimationFrame(tick);
  }
  const api = {
    get t() { return p.t; }, get playing() { return p.playing; }, get guides() { return overlay.kind !== 'none'; },
    async play() {
      if (opts.beforePlay) { const ok = await opts.beforePlay(); if (!ok) { sync(); return; } }
      const T = opts.total(); if (p.t >= T - 1e-3) p.t = 0;
      p.t0 = performance.now() - p.t * 1000; p.playing = true;
      cancelAnimationFrame(p.raf); p.raf = requestAnimationFrame(tick); sync();
      opts.onPlay?.(p.t);
    },
    pause() { const was = p.playing; p.playing = false; cancelAnimationFrame(p.raf); sync(); if (was) opts.onPause?.(p.t); },
    toggle() { p.playing ? api.pause() : api.play(); },
    seek(t) { p.t = clamp(t, 0, opts.total()); if (p.playing) p.t0 = performance.now() - p.t * 1000; opts.render(p.t); sync(); },
    sync,
  };
  els.play.addEventListener('click', api.toggle);
  els.rew.addEventListener('click', () => api.seek(0));
  els.scrub.addEventListener('input', () => { if (p.playing) api.pause(); api.seek(parseFloat(els.scrub.value)); });
  els.overlay.value = overlay.kind;
  els.overlay.addEventListener('change', () => setOverlay(els.overlay.value));
  overlay.players.push({ select: els.overlay, redraw: () => opts.render(p.t) });
  if (opts.track) {
    opts.track.addEventListener('pointerdown', e => {
      const r = opts.track.getBoundingClientRect();
      if (p.playing) api.pause();
      api.seek(((e.clientX - r.left) / r.width) * opts.total());
    });
  }
  return api;
}

/* Fit a viewer box of a given aspect into its monitor */
function fitViewer(monitor, viewer, aspect) {
  const [w, h] = FRAMES[aspect];
  const cs = getComputedStyle(monitor);
  const aw = monitor.clientWidth - parseFloat(cs.paddingLeft) - parseFloat(cs.paddingRight);
  const narrow = window.matchMedia('(max-width: 900px)').matches;
  const ah = narrow ? window.innerHeight * 0.62 : monitor.clientHeight - parseFloat(cs.paddingTop) - parseFloat(cs.paddingBottom);
  const s = Math.max(0.01, Math.min(aw / w, ah / h));
  const vw = Math.max(40, Math.floor(w * s)), vh = Math.max(40, Math.floor(h * s));
  viewer.style.width = vw + 'px'; viewer.style.height = vh + 'px';
  return { vw, vh };
}

/* Preview overlay, shared by every tool: none, safe zones, or a platform's interface (TikTok, Instagram Reels,
   YouTube Shorts; drawn by platforms.js on 9:16 frames). Never part of a render: tools draw it on their guide canvas. */
const OVERLAY_KINDS = { none: 'No overlay', safe: 'Safe zones', tiktok: 'TikTok', reels: 'Instagram Reels', shorts: 'YouTube Shorts' };
const OVERLAY_OPTIONS = Object.entries(OVERLAY_KINDS).map(([v, l]) => `<option value="${v}">${l}</option>`).join('');
const overlay = { kind: OVERLAY_KINDS[store.get('overlay')] ? store.get('overlay') : 'none', players: [] };
function setOverlay(kind) {
  if (!OVERLAY_KINDS[kind]) kind = 'none';
  overlay.kind = kind;
  store.set('overlay', kind);
  for (const pl of overlay.players) { pl.select.value = kind; pl.redraw(); }
}

/* Guides for the preview: a platform's interface on 9:16 frames, otherwise the title-safe box plus the area
   vertical platforms cover with buttons and captions */
function drawGuides(ctx, W, H, aspect, s) {
  const kind = overlay.kind;
  if (kind === 'none') return;
  if (kind !== 'safe' && aspect === '9:16' && window.CS.drawPlatformUI) { window.CS.drawPlatformUI(ctx, W, H, kind); return; }
  ctx.save();
  ctx.lineWidth = Math.max(1, s);
  ctx.setLineDash([6 * s, 5 * s]);
  ctx.strokeStyle = 'rgba(244,196,67,0.85)';
  ctx.strokeRect(W * 0.05, H * 0.05, W * 0.9, H * 0.9);
  ctx.setLineDash([]);
  ctx.font = `600 ${Math.round(11 * s)}px Barlow, Arial, sans-serif`;
  if (aspect === '9:16' || aspect === '4:5') {
    ctx.fillStyle = 'rgba(255,75,62,0.16)';
    ctx.fillRect(0, H * 0.8, W, H * 0.2);
    ctx.fillRect(W * 0.86, H * 0.42, W * 0.14, H * 0.38);
    ctx.fillRect(0, 0, W, H * 0.08);
    ctx.fillStyle = 'rgba(255,255,255,0.75)';
    ctx.fillText('Platform buttons and captions', W * 0.05, H * 0.8 + 18 * s);
  }
  ctx.fillStyle = 'rgba(244,196,67,0.9)';
  ctx.fillText(kind === 'safe' ? 'Title safe' : `Title safe · the ${OVERLAY_KINDS[kind]} overlay is for 9:16 frames`, W * 0.05 + 6 * s, H * 0.05 + 15 * s);
  ctx.restore();
}

/* ============================================================
   Video export (WebCodecs H.264 → MP4, or PNG frames → ZIP)
   ============================================================ */
async function pickEncoder(W, H, fps) {
  const bitrate = Math.min(80e6, Math.round(W * H * fps * 0.26));
  const avc = ['avc1.640034', 'avc1.640033', 'avc1.64002A', 'avc1.640028', 'avc1.4D0034', 'avc1.4D0028', 'avc1.42E034', 'avc1.42E01F'];
  for (const codec of avc) {
    const config = { codec, width: W, height: H, bitrate, framerate: fps, avc: { format: 'avc' } };
    try { const s = await VideoEncoder.isConfigSupported(config); if (s.supported) return { config, mux: 'avc', label: 'H.264' }; } catch (e) { /* try next */ }
  }
  for (const codec of ['vp09.00.51.08', 'vp09.00.41.08', 'vp09.00.10.08']) {
    const config = { codec, width: W, height: H, bitrate, framerate: fps };
    try { const s = await VideoEncoder.isConfigSupported(config); if (s.supported) return { config, mux: 'vp9', label: 'VP9' }; } catch (e) { /* try next */ }
  }
  throw new Error('This browser has no video encoder for this size. Try HD instead of 4K, use Chrome or Edge, or choose PNG frames.');
}
/* Audio: pick an encoder (AAC preferred for editing apps, Opus as fallback) and feed it a mixed AudioBuffer */
async function pickAudioEncoder(sampleRate, channels) {
  if (!('AudioEncoder' in window)) return null;
  for (const [codec, mux, label] of [['mp4a.40.2', 'aac', 'AAC'], ['opus', 'opus', 'Opus']]) {
    const config = { codec, sampleRate, numberOfChannels: channels, bitrate: 192000 };
    try { const s = await AudioEncoder.isConfigSupported(config); if (s.supported) return { config, mux, label }; } catch (e) { /* try next */ }
  }
  return null;
}
async function encodeAudio(muxer, buffer, config) {
  let err = null;
  const enc = new AudioEncoder({ output: (chunk, meta) => muxer.addAudioChunk(chunk, meta), error: e => { err = e; } });
  enc.configure(config);
  const sr = buffer.sampleRate, ch = buffer.numberOfChannels, total = buffer.length, step = 4800;
  const chans = Array.from({ length: ch }, (_, c) => buffer.getChannelData(c));
  for (let i = 0; i < total; i += step) {
    if (err) throw err;
    const n = Math.min(step, total - i);
    const data = new Float32Array(n * ch);
    chans.forEach((c, k) => data.set(c.subarray(i, i + n), k * n));
    const ad = new AudioData({ format: 'f32-planar', sampleRate: sr, numberOfFrames: n, numberOfChannels: ch, timestamp: Math.round(i / sr * 1e6), data });
    enc.encode(ad); ad.close();
    while (enc.encodeQueueSize > 8) await sleep(1);
  }
  await enc.flush(); enc.close();
  if (err) throw err;
}
function encodeWav(buffer) {
  const ch = buffer.numberOfChannels, sr = buffer.sampleRate, n = buffer.length;
  const out = new DataView(new ArrayBuffer(44 + n * ch * 2));
  const str = (o, t) => { for (let i = 0; i < t.length; i++) out.setUint8(o + i, t.charCodeAt(i)); };
  str(0, 'RIFF'); out.setUint32(4, 36 + n * ch * 2, true); str(8, 'WAVE'); str(12, 'fmt ');
  out.setUint32(16, 16, true); out.setUint16(20, 1, true); out.setUint16(22, ch, true); out.setUint32(24, sr, true);
  out.setUint32(28, sr * ch * 2, true); out.setUint16(32, ch * 2, true); out.setUint16(34, 16, true); str(36, 'data'); out.setUint32(40, n * ch * 2, true);
  const chans = Array.from({ length: ch }, (_, c) => buffer.getChannelData(c));
  let o = 44;
  for (let i = 0; i < n; i++) for (let c = 0; c < ch; c++) { const v = clamp(chans[c][i], -1, 1); out.setInt16(o, v < 0 ? v * 0x8000 : v * 0x7fff, true); o += 2; }
  return new Uint8Array(out.buffer);
}
async function mp4Sink(W, H, fps, audio = null) {
  if (!('VideoEncoder' in window)) throw new Error('This browser can’t encode video. Use Chrome, Edge, or Safari 17+, or choose PNG frames.');
  if (!window.Mp4Muxer) throw new Error('The MP4 writer didn’t load. Check your internet connection and reload.');
  const pick = await pickEncoder(W, H, fps);
  const aPick = audio ? await pickAudioEncoder(audio.sampleRate, audio.numberOfChannels) : null;
  const muxer = new Mp4Muxer.Muxer({
    target: new Mp4Muxer.ArrayBufferTarget(),
    video: { codec: pick.mux, width: W, height: H, frameRate: fps },
    ...(aPick ? { audio: { codec: aPick.mux, numberOfChannels: audio.numberOfChannels, sampleRate: audio.sampleRate } } : {}),
    fastStart: 'in-memory', firstTimestampBehavior: 'offset',
  });
  let err = null;
  const enc = new VideoEncoder({ output: (chunk, meta) => muxer.addVideoChunk(chunk, meta), error: e => { err = e; } });
  enc.configure(pick.config);
  const dur = Math.round(1e6 / fps);
  const label = pick.label === 'VP9' ? 'VP9 video (this browser has no H.264 encoder)' : 'H.264 video';
  return {
    ext: 'mp4',
    label: label + (aPick ? ` + ${aPick.label} audio` : audio ? ' · audio skipped: this browser has no audio encoder' : ''),
    async add(canvas, i) {
      if (err) throw err;
      while (enc.encodeQueueSize > 4) await sleep(2);
      const frame = new VideoFrame(canvas, { timestamp: Math.round(i * 1e6 / fps), duration: dur });
      enc.encode(frame, { keyFrame: i % (fps * 2) === 0 });
      frame.close();
    },
    async finish() {
      await enc.flush(); if (err) throw err;
      if (aPick) await encodeAudio(muxer, audio, aPick.config);
      muxer.finalize(); enc.close();
      return new Blob([muxer.target.buffer], { type: 'video/mp4' });
    },
    abort() { try { enc.close(); } catch (e) { /* already closed */ } },
  };
}
function pngSink(audio = null) {
  if (!window.fflate) throw new Error('The ZIP writer didn’t load. Check your internet connection and reload.');
  const files = {};
  return {
    ext: 'zip', label: audio ? 'PNG frames + audio.wav' : 'PNG frames',
    async add(canvas, i) {
      const blob = await new Promise(r => canvas.toBlob(r, 'image/png'));
      files[`frame_${String(i).padStart(5, '0')}.png`] = new Uint8Array(await blob.arrayBuffer());
    },
    async finish() {
      if (audio) files['audio.wav'] = encodeWav(audio);
      return new Blob([fflate.zipSync(files, { level: 0 })], { type: 'application/zip' });
    },
    abort() {},
  };
}
/* Frames for the host app: PNG when the first frame has any transparency (a lower third over the video),
   JPEG otherwise (full-frame graphics, far smaller) */
function hasAlpha(canvas) {
  const d = canvas.getContext('2d').getImageData(0, 0, canvas.width, canvas.height).data;
  for (let i = 3; i < d.length; i += 16) if (d[i] < 255) return true;
  return false;
}
async function hostSink(W, H, fps, total, audio, name) {
  const job = await HOST.begin({ name, width: W, height: H, fps });
  let ext = null, n = 0;
  return {
    ext: 'clip', label: 'Added to your video',
    async add(canvas, i) {
      if (!ext) ext = hasAlpha(canvas) ? 'png' : 'jpg';
      const blob = await new Promise(r => canvas.toBlob(r, ext === 'png' ? 'image/png' : 'image/jpeg', 0.92));
      await HOST.frame(job, i, new Uint8Array(await blob.arrayBuffer()), ext);
      n = i + 1;
    },
    async finish() {
      if (audio) await HOST.audio(job, encodeWav(audio));
      await HOST.finish(job, { ...render.toHost, name, width: W, height: H, fps, frames: n, seconds: total, ext });
      return null;
    },
    abort() { try { HOST.abort(job); } catch (_) { /* ignore */ } },
  };
}
/* Add to video: the tool's own render, its frames going to the app. Frames keep their transparency. */
async function addToVideo(prefix) {
  const o = outputs[prefix];
  if (!o || !HOST || render.busy) return 'busy';
  const format = o.out.format;
  o.out.format = 'png-seq';
  render.toHost = { tool: prefix, state: snapshot(prefix), edit: hosted.editing };
  let status;
  try { status = await o.actions.render(); } finally { o.out.format = format; render.toHost = null; }
  // added: the next one is a new graphic
  if (status === 'done') hosted.open(prefix, null, null);
  return status;
}
/* A tool's settings and content as plain JSON (no media files), for the app to keep with its clip */
function snapshot(prefix) { const r = restorers[prefix]; return r ? clone(r.state) : null; }
function restore(prefix, saved) {
  const r = restorers[prefix];
  if (!r || !saved || typeof saved !== 'object') return false;
  for (const [k, v] of Object.entries(saved)) {
    if (!(k in r.state)) continue;
    if (k === 'out' && r.state.out && v && typeof v === 'object') Object.assign(r.state.out, clone(v));
    else r.state[k] = clone(v);
  }
  resync(r.state); if (r.state.out) resync(r.state.out);
  r.afterApply();
  return true;
}
/* What the host app calls: match its video's shape, open a graphic again to change it, show its video
   behind the preview */
const backdropFns = [];
const hosted = {
  active: !!HOST,
  editing: null,
  // the app's video at its playhead, an <img>: tools with a preview-only background (Lower Thirds) show
  // the graphic over it; never part of a render
  backdrop: null,
  async setBackdrop(src) {
    let img = null;
    if (src && typeof src === 'object' && 'naturalWidth' in src) img = src;
    else if (typeof src === 'string' && src) {
      img = new Image();
      try { img.src = src; await img.decode(); } catch (_) { img = null; }
    }
    hosted.backdrop = img;
    for (const fn of backdropFns) { try { fn(img); } catch (_) { /* a tool's own problem */ } }
    return !!img;
  },
  onBackdrop(fn) { backdropFns.push(fn); },
  setAspect(a) {
    for (const o of Object.values(outputs)) {
      if (!o.aspects.includes(a) || o.out.aspect === a) continue;
      o.out.aspect = a; resync(o.out); o.onChange('out.aspect');
    }
  },
  // `edit` is the app's own handle for the clip being changed (it comes back in finish); null for a new
  // one. With no `prefix`, the tool on screen stays and the next Add to video makes a new graphic.
  open(prefix, saved, edit = null) {
    if (prefix) {
      window.CS.showTool?.(prefix);
      // a box still being typed in keeps its text; the graphic's own comes back everywhere
      document.activeElement?.blur?.();
      if (saved) restore(prefix, saved);
    }
    hosted.editing = edit;
    for (const p of Object.keys(outputs)) { const b = $(`#${p}-add`); if (b) b.textContent = edit != null && p === prefix ? 'Update in video' : 'Add to video'; }
  },
  snapshot, restore, add: addToVideo,
};
function downloadBlob(blob, name) {
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url; a.download = name; document.body.appendChild(a); a.click(); a.remove();
  return url;
}
function fmtBytes(n) { return n > 1e9 ? (n / 1e9).toFixed(2) + ' GB' : n > 1e6 ? (n / 1e6).toFixed(1) + ' MB' : Math.round(n / 1e3) + ' KB'; }

const render = { busy: false, cancel: false, url: null, name: '', toHost: null };
async function runRender({ title, OW, OH, fps, total, format, name, setup, frame, teardown, getAudio }) {
  if (render.busy) return 'busy';
  const dlg = $('#render-dlg'), stage = $('#render-stage');
  render.busy = true; render.cancel = false;
  $('#render-title').textContent = title;
  $('#render-bar').style.width = '0%';
  $('#render-count').textContent = 'Preparing…'; $('#render-eta').textContent = '';
  $('#render-note').textContent = 'Keep this tab in front while rendering. Browsers pause background tabs.';
  $('#render-cancel').hidden = false; $('#render-close').hidden = true; $('#render-save').hidden = true;
  $$('canvas', stage).forEach(c => c.remove());
  const host = $('#render-maphost');
  host.innerHTML = ''; host.style.transform = '';
  stage.style.aspectRatio = `${OW} / ${OH}`;
  stage.style.maxHeight = '52vh';
  stage.style.width = OH > OW ? `min(100%, calc(52vh * ${OW / OH}))` : '100%';
  stage.style.margin = '0 auto';
  if (!dlg.open) dlg.showModal();
  const canvas = document.createElement('canvas');
  canvas.width = OW; canvas.height = OH;
  stage.appendChild(canvas);
  const ctx = canvas.getContext('2d');
  let sink = null, status = 'done';
  try {
    await setup?.({ stage, host });
    let audio = null;
    if (getAudio) { $('#render-count').textContent = 'Mixing audio…'; audio = await getAudio(); }
    const N = Math.max(1, Math.round(total * fps));
    sink = render.toHost ? await hostSink(OW, OH, fps, total, audio, name) : format === 'png-seq' ? pngSink(audio) : await mp4Sink(OW, OH, fps, audio);
    const started = performance.now();
    for (let i = 0; i < N; i++) {
      if (render.cancel) break;
      const t = i / fps;
      await frame(ctx, t, i);
      await sink.add(canvas, i);
      const done = i + 1;
      $('#render-bar').style.width = (done / N * 100).toFixed(1) + '%';
      const el = (performance.now() - started) / 1000;
      $('#render-count').textContent = `Frame ${done} / ${N}`;
      $('#render-eta').textContent = done > 3 ? `about ${Math.max(0, Math.round(el / done * (N - done)))}s left` : '';
      if (i % 4 === 0) await sleep(0);
    }
    if (render.cancel) { sink.abort(); dlg.close(); toast('Render cancelled.'); return 'cancel'; }
    $('#render-count').textContent = 'Finishing file…'; $('#render-eta').textContent = '';
    const blob = await sink.finish();
    if (!blob) { dlg.close(); toast('Added to your video.'); return 'done'; }
    if (render.url) URL.revokeObjectURL(render.url);
    render.name = `${name}.${sink.ext}`;
    render.url = downloadBlob(blob, render.name);
    $('#render-title').textContent = 'Done';
    $('#render-count').textContent = `${render.name} · ${fmtBytes(blob.size)}`;
    $('#render-eta').textContent = sink.label;
    $('#render-note').textContent = 'Saved to your Downloads folder. If nothing appeared, use Download again.';
    $('#render-cancel').hidden = true; $('#render-close').hidden = false; $('#render-save').hidden = false;
  } catch (e) {
    console.error(e);
    status = 'error';
    try { sink?.abort(); } catch (_) { /* ignore */ }
    $('#render-title').textContent = 'Render stopped';
    $('#render-count').textContent = e.message || String(e);
    $('#render-note').textContent = '';
    $('#render-cancel').hidden = true; $('#render-close').hidden = false;
  } finally {
    try { await teardown?.(); } catch (_) { /* ignore */ }
    render.busy = false;
  }
  return status;
}
$('#render-cancel').addEventListener('click', () => { render.cancel = true; });
$('#render-close').addEventListener('click', () => $('#render-dlg').close());
$('#render-save').addEventListener('click', () => { if (render.url) { const a = document.createElement('a'); a.href = render.url; a.download = render.name; a.click(); } });
$('#render-dlg').addEventListener('cancel', e => { if (render.busy) { e.preventDefault(); render.cancel = true; } });

/* Render the same clip as 16:9 and 9:16, one after the other */
async function renderBoth(out, refresh, renderOne) {
  const orig = out.aspect;
  const names = [];
  for (const a of ['16:9', '9:16']) {
    out.aspect = a; resync(out); refresh();
    const st = await renderOne();
    if (st !== 'done') break;
    names.push(render.name);
  }
  out.aspect = orig; resync(out); refresh();
  if (names.length === 2) {
    $('#render-title').textContent = 'Done: 2 files';
    $('#render-count').textContent = names.join(' + ');
    $('#render-note').textContent = 'Both saved to Downloads. If your browser asks to allow multiple downloads, allow it, then use Download again for the second file.';
  }
}

async function saveStill(canvas, name) {
  const blob = await new Promise(r => canvas.toBlob(r, 'image/png'));
  downloadBlob(blob, name);
  toast(`Saved ${name}`);
}

window.CS = Object.assign(window.CS || {}, {
  tools: {},
  $,
  $$,
  ASPECT_LABELS,
  PRESET_TOOLS,
  pickAudioEncoder,
  encodeAudio,
  encodeWav,
  clamp,
  lerp,
  smoothstep,
  sleep,
  EASE,
  ease,
  easeOutBounce,
  easeOutBack,
  mulberry32,
  pad2,
  fmtTC,
  fmtSec,
  isHex,
  slug,
  clone,
  store,
  toastTimer,
  toast,
  roundRect,
  loadImageFile,
  FRAMES,
  dims,
  SWATCHES,
  registry,
  reg,
  resync,
  saveTimers,
  saveSoon,
  bindRange,
  bindColor,
  bindCheck,
  bindSeg,
  bindText,
  bindSelect,
  buildOutputPanel,
  userPresets,
  presetUI,
  buildPresetPanel,
  ICON_PLAY,
  ICON_PAUSE,
  ICON_START,
  makePlayer,
  fitViewer,
  drawGuides,
  OVERLAY_KINDS,
  setOverlay,
  getOverlay: () => overlay.kind,
  pickEncoder,
  mp4Sink,
  pngSink,
  downloadBlob,
  fmtBytes,
  render,
  runRender,
  hostSink,
  hosted,
  lib,
  renderBoth,
  saveStill,
});
})();
