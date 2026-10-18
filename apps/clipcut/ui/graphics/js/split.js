/* Clip Studio · Split Screen tool
   Classic script (not an ES module) so index.html still works when opened straight from disk.
   Shared helpers live on window.CS; each tool registers itself in CS.tools.

   Layout: 16:9 frames split into 1–3 columns, 9:16 frames into 1–4 rows; four panels can also sit in a 2 × 2 grid
   (in 16:9, four panels always use the grid). Two panels can also be a before/after wipe: both fill the frame and
   a divider sweeps across to reveal panel 2 over panel 1.
   Each panel holds a video or image. Video frames come from <video> elements: the preview lets them play,
   the export seeks each one to the exact time of every frame. Audio is mixed offline (OfflineAudioContext)
   and handed to the shared MP4 writer. */
(() => {
'use strict';
const {
  $, $$, clamp, lerp, sleep, EASE, easeOutBack, fmtSec, slug, clone, store, toast, roundRect, loadImageFile,
  FRAMES, dims, resync, saveSoon, bindRange, bindColor, bindCheck, bindSeg, bindSelect,
  buildOutputPanel, buildPresetPanel, makePlayer, fitViewer, drawGuides, runRender, saveStill,
} = window.CS;

const LABEL_STACK = '"Libre Franklin", "Helvetica Neue", Arial, sans-serif';
const MAX_PANELS = 4, MAX_ACROSS = 3;  // four side-by-side columns are too narrow, so 16:9 uses the grid for four
const blankPanel = () => ({ fit: 'fill', zoom: 1, posX: 0.5, posY: 0.5, motion: 'none', trimIn: 0, loop: true, volume: 1, label: '' });
const SP_DEFAULT = {
  count: 3, layout: 'stack', wipeMode: 'compare', wipeDir: 'lr', wipeHold: 1, wipeDur: 1.6, wipeColor: '#FFFFFF', wipeHandle: true, gap: 12, margin: 0, radius: 0, bg: '#000000', panelBg: '#161514',
  entrance: 'slide', delay: 0.2, stagger: 0.25, entDur: 0.55, motionAmt: 12,
  spotlight: false, spotDur: 3, spotMatch: true, spotPlay: true, spotColor: '#F4C443',
  sound: 'one', soundPanel: 0,
  badges: true, labelPos: 'bottom', labelStyle: 'pill', labelSize: 30, labelBg: '#FFFFFF', labelFg: '#111111',
  autoDur: true, duration: 8,
  panels: [blankPanel(), blankPanel(), blankPanel(), blankPanel()],
  out: { aspect: '16:9', quality: '1080', fps: 30, format: 'mp4' },
};
const SP_PRESET_KEYS = ['count', 'layout', 'wipeMode', 'wipeDir', 'wipeHold', 'wipeDur', 'wipeColor', 'wipeHandle', 'gap', 'margin', 'radius', 'bg', 'panelBg', 'entrance', 'delay', 'stagger', 'entDur', 'motionAmt', 'spotlight', 'spotDur', 'spotMatch', 'spotPlay', 'spotColor', 'sound', 'badges', 'labelPos', 'labelStyle', 'labelSize', 'labelBg', 'labelFg', 'autoDur', 'duration', 'out'];
const SP_BUILTINS = {
  'Trend triptych (16:9)': { count: 3, layout: 'stack', gap: 12, margin: 0, radius: 0, bg: '#000000', entrance: 'slide', stagger: 0.25, entDur: 0.55, spotlight: false, sound: 'one', badges: true, labelPos: 'bottom', labelStyle: 'pill', out: { aspect: '16:9' } },
  'Walkthrough with spotlight (16:9)': { count: 3, layout: 'stack', gap: 18, margin: 28, radius: 18, bg: '#0E0D0C', entrance: 'pop', stagger: 0.2, spotlight: true, spotMatch: true, spotPlay: true, sound: 'spot', spotColor: '#F4C443', badges: true, labelPos: 'bottom', labelStyle: 'pill', out: { aspect: '16:9' } },
  'Vertical stack (9:16)': { count: 3, layout: 'stack', gap: 10, margin: 0, radius: 0, bg: '#000000', entrance: 'wipe', stagger: 0.2, entDur: 0.5, spotlight: false, badges: false, labelPos: 'top', labelStyle: 'bar', out: { aspect: '9:16' } },
  'Before / after (9:16)': { count: 2, layout: 'stack', gap: 8, margin: 0, radius: 0, bg: '#000000', entrance: 'slide', stagger: 0.3, spotlight: false, badges: false, labelPos: 'top', labelStyle: 'pill', out: { aspect: '9:16' } },
  '2 × 2 grid (9:16)': { count: 4, layout: 'grid', gap: 8, margin: 0, radius: 0, bg: '#000000', entrance: 'pop', stagger: 0.15, entDur: 0.5, spotlight: false, badges: true, labelPos: 'bottom', labelStyle: 'pill', labelSize: 26, out: { aspect: '9:16' } },
  '2 × 2 grid (16:9)': { count: 4, layout: 'grid', gap: 8, margin: 0, radius: 0, bg: '#000000', entrance: 'pop', stagger: 0.15, entDur: 0.5, spotlight: false, badges: true, labelPos: 'bottom', labelStyle: 'pill', labelSize: 26, out: { aspect: '16:9' } },
  'Before / after wipe': { count: 2, layout: 'wipe', wipeMode: 'compare', wipeDir: 'lr', wipeHold: 1, wipeDur: 1.6, wipeColor: '#FFFFFF', wipeHandle: true, margin: 0, radius: 0, entrance: 'fade', delay: 0, entDur: 0.4, spotlight: false, badges: false, labelPos: 'top', labelStyle: 'pill' },
  'Clean grid': { gap: 0, margin: 0, radius: 0, entrance: 'none', badges: false, spotlight: false },
};

const sp = Object.assign(clone(SP_DEFAULT), store.get('sp', {}));
sp.out = Object.assign({}, SP_DEFAULT.out, sp.out || {});
if (!['16:9', '9:16'].includes(sp.out.aspect)) sp.out.aspect = '16:9';
sp.panels = Array.from({ length: 4 }, (_, i) => Object.assign(blankPanel(), (sp.panels || [])[i] || {}));
sp.panels.forEach(P => { P.trimIn = 0; }); // trims belong to a specific file, which isn't kept between sessions

const media = [null, null, null, null];   // { type, el, w, h, duration, name, audio, sample }
let sel = 0, exporting = false, spTL = null;

const toolSp = $('#tool-sp');
const spCanvas = $('#sp-canvas'), spGuides = $('#sp-guides'), spViewer = $('#sp-viewer'), spMonitor = $('#sp-monitor');
const mediaHost = $('#sp-media');
const isVertical = () => sp.out.aspect === '9:16';
const spCount = () => clamp(Math.round(sp.count), 1, MAX_PANELS);
// The 2 × 2 grid only exists for four panels; in 16:9 four panels always use it
const isGrid = () => sp.layout === 'grid' && spCount() === 4;
const isWipe = () => sp.layout === 'wipe' && spCount() === 2;
const layoutFor = () => (sp.count === 2 && sp.layout === 'wipe' ? 'wipe' : sp.count !== 4 ? 'stack' : isVertical() ? (sp.layout === 'grid' ? 'grid' : 'stack') : 'grid');
sp.layout = layoutFor();

/* ---------- sample media (clearly marked) so the first view shows the layout ---------- */
function makeSample(i) {
  const c = document.createElement('canvas'); c.width = 1080; c.height = 1350;
  const g = c.getContext('2d');
  const hues = [8, 205, 275, 150][i];
  const grad = g.createLinearGradient(0, 0, 1080, 1350);
  grad.addColorStop(0, `hsl(${hues} 70% 52%)`); grad.addColorStop(1, `hsl(${hues + 40} 65% 22%)`);
  g.fillStyle = grad; g.fillRect(0, 0, 1080, 1350);
  const rnd = (n => () => (n = (n * 16807) % 2147483647) / 2147483647)(97 + i * 31);
  for (let k = 0; k < 14; k++) { g.globalAlpha = 0.08 + rnd() * 0.12; g.fillStyle = '#fff'; g.beginPath(); g.arc(rnd() * 1080, rnd() * 1350, 60 + rnd() * 240, 0, Math.PI * 2); g.fill(); }
  g.globalAlpha = 1; g.fillStyle = 'rgba(255,255,255,0.92)'; g.textAlign = 'center';
  g.font = '700 190px "Barlow Condensed", "Arial Narrow", Arial, sans-serif'; g.fillText(String(i + 1), 540, 690);
  g.font = '600 54px "Barlow", Arial, sans-serif'; g.fillText('SAMPLE', 540, 790);
  g.font = '500 38px "Barlow", Arial, sans-serif'; g.fillStyle = 'rgba(255,255,255,0.75)'; g.fillText('Drop your video or image here', 540, 860);
  return { type: 'image', el: c, w: c.width, h: c.height, name: `Sample ${i + 1}`, sample: true };
}

/* ---------- loading media ---------- */
function videoDuration(v) {
  if (isFinite(v.duration) && v.duration > 0) return Promise.resolve(v.duration);
  return new Promise(res => {  // some recordings report Infinity until you seek past the end
    const done = () => { v.removeEventListener('durationchange', check); res(isFinite(v.duration) ? v.duration : 1); v.currentTime = 0; };
    const check = () => { if (isFinite(v.duration)) done(); };
    v.addEventListener('durationchange', check);
    v.currentTime = 1e9;
    setTimeout(done, 4000);
  });
}
async function loadMedia(file) {
  if (/^image\//.test(file.type)) {
    const img = await loadImageFile(file);
    return { type: 'image', el: img, w: img.naturalWidth, h: img.naturalHeight, name: file.name };
  }
  if (/^video\//.test(file.type) || /\.(mp4|m4v|mov|webm|mkv)$/i.test(file.name)) {
    const url = URL.createObjectURL(file);
    const v = document.createElement('video');
    v.preload = 'auto'; v.playsInline = true; v.src = url;
    mediaHost.appendChild(v);
    await new Promise((res, rej) => {
      const tm = setTimeout(() => rej(new Error(`“${file.name}” took too long to open.`)), 30000);
      v.addEventListener('loadeddata', () => { clearTimeout(tm); res(); }, { once: true });
      v.addEventListener('error', () => { clearTimeout(tm); rej(new Error(`This browser can’t play “${file.name}”. Convert it to MP4 (H.264) and try again.`)); }, { once: true });
    });
    const duration = await videoDuration(v);
    const m = { type: 'video', el: v, w: v.videoWidth, h: v.videoHeight, duration, name: file.name, url, audio: null, audioState: 'reading' };
    v.addEventListener('seeked', () => { if (!exporting && !spPlayer.playing) spPaint(spPlayer.t); });
    // Decode the soundtrack for the export mix (the preview plays sound straight from the video element)
    file.arrayBuffer()
      .then(buf => new OfflineAudioContext(2, 1, 48000).decodeAudioData(buf))
      .then(ab => { m.audio = ab; m.audioState = 'ok'; })
      .catch(() => { m.audio = null; m.audioState = 'none'; })
      .finally(() => spBuildCards());
    return m;
  }
  throw new Error(`“${file.name}” isn’t a video or image.`);
}
function disposeMedia(m) {
  if (!m) return;
  if (m.type === 'video') { m.el.pause(); m.el.removeAttribute('src'); m.el.load(); m.el.remove(); URL.revokeObjectURL(m.url); }
  else if (m.el.src && m.el.src.startsWith('blob:')) URL.revokeObjectURL(m.el.src);
}
async function setMedia(i, file) {
  try {
    toast(`Loading ${file.name}…`, 2000);
    const m = await loadMedia(file);
    disposeMedia(media[i]);
    media[i] = m;
    const P = sp.panels[i];
    P.trimIn = 0; P.posX = 0.5; P.posY = 0.5; P.zoom = 1;
    if (m.type === 'video' && sp.sound === 'one' && !media.some((x, k) => k !== i && x && x.type === 'video')) sp.soundPanel = i;
    sel = i;
    spBuildCards(); spRefresh();
    spPlayer.seek(Math.min(spPlayer.t, spTL.total));
  } catch (e) { toast(e.message || String(e)); }
}
async function setMany(files, start = 0) {
  const list = [...files].filter(f => /^(image|video)\//.test(f.type) || /\.(mp4|m4v|mov|webm|mkv)$/i.test(f.name));
  if (!list.length) { toast('Drop video or image files.'); return; }
  const take = list.slice(0, MAX_PANELS - start);
  if (start + take.length > spCount()) { bindCount.set(Math.min(MAX_PANELS, start + take.length)); bindLayout.set(layoutFor()); }
  for (let k = 0; k < take.length; k++) await setMedia(start + k, take[k]);
  if (list.length > take.length) toast(`Only ${MAX_PANELS} panels fit; extra files were skipped.`);
}

/* ---------- timeline ---------- */
function spTimeline() {
  const n = spCount(), wipe = isWipe();
  const ent = sp.entrance !== 'none';
  // In a wipe both panels share one frame, so they enter together
  const appear = Array.from({ length: n }, (_, i) => (ent ? sp.delay + (wipe ? 0 : i * sp.stagger) : 0));
  const entDur = ent ? Math.max(0.05, sp.entDur) : 0;
  const allIn = appear[n - 1] + entDur;
  const clipLen = i => { const m = media[i]; return m && m.type === 'video' ? Math.max(0.1, m.duration - sp.panels[i].trimIn) : null; };
  const spots = [];
  if (sp.spotlight && !wipe) {
    let t = allIn + 0.3;
    for (let i = 0; i < n; i++) {
      const len = sp.spotMatch && clipLen(i) ? Math.min(clipLen(i), 60) : sp.spotDur;
      spots.push({ i, t0: t, t1: t + len }); t += len;
    }
  }
  const spotPlay = spots.length > 0 && sp.spotPlay;
  // Wipe: hold on panel 1, sweep to show all of panel 2; "compare" then settles halfway so both show
  const w0 = allIn + sp.wipeHold, wd = Math.max(0.2, sp.wipeDur);
  const wipeEnd = !wipe ? 0 : sp.wipeMode === 'compare' ? w0 + wd + 0.6 + wd * 0.6 : w0 + wd;
  const wipePos = t => {
    if (!wipe) return 0;
    const a = EASE.smooth(clamp((t - w0) / wd, 0, 1));
    if (sp.wipeMode !== 'compare') return a;
    return a - 0.5 * EASE.smooth(clamp((t - w0 - wd - 0.6) / (wd * 0.6), 0, 1));
  };
  let total;
  if (sp.autoDur) {
    total = Math.max(allIn + 2.5, wipe ? wipeEnd + 1.5 : 0);
    if (spots.length) total = Math.max(total, spots[spots.length - 1].t1 + 0.8);
    else for (let i = 0; i < n; i++) { const L = clipLen(i); if (L) total = Math.max(total, appear[i] + L); }
    total = clamp(total, 1, 180);
  } else total = sp.duration;
  const advancing = (i, t) => {
    const L = clipLen(i); if (!L) return false;
    if (spotPlay) { const s = spots[i]; return t >= s.t0 && t < s.t1 && t - s.t0 < L; }
    return t >= appear[i] && (sp.panels[i].loop || t - appear[i] < L);
  };
  const videoTime = (i, t) => {
    const P = sp.panels[i], L = clipLen(i) || 0.1;
    let rel = spotPlay ? clamp(t - spots[i].t0, 0, spots[i].t1 - spots[i].t0) : Math.max(0, t - appear[i]);
    rel = !spotPlay && P.loop ? rel % L : Math.min(rel, Math.max(0, L - 0.04));
    return P.trimIn + rel;
  };
  return { n, appear, entDur, allIn, spots, spotPlay, total, clipLen, advancing, videoTime, wipe, w0, wipeEnd, wipePos };
}
function gainAt(i, t, TL) {
  const vol = sp.panels[i].volume;
  switch (sp.sound) {
    case 'none': return 0;
    case 'one': return i === sp.soundPanel ? vol : 0;
    case 'spot': {
      if (!TL.spots.length) return vol;
      const w = TL.spots.find(s => t >= s.t0 && t < s.t1);
      return w && w.i === i ? vol : 0;
    }
    default: return vol;
  }
}
const lift = (t, t0, t1, r = 0.22) => clamp((t - t0) / r, 0, 1) * clamp((t1 - t) / r, 0, 1);
function spotLevels(t, TL) {
  if (!TL.spots.length) return null;
  const lit = TL.spots.map(s => lift(t, s.t0, s.t1));
  const any = Math.max(...lit);
  return { lit, dim: lit.map(v => Math.max(0, any - v)) };
}

/* ---------- drawing ---------- */
function spRects(W, H, n) {
  const m = sp.margin, g = sp.gap, out = [];
  if (n === 2 && sp.layout === 'wipe') {
    const R = { x: m, y: m, w: W - 2 * m, h: H - 2 * m };
    return [R, { ...R }];
  } else if (n === 4 && sp.layout === 'grid') {
    const w = (W - 2 * m - g) / 2, h = (H - 2 * m - g) / 2;
    for (let i = 0; i < n; i++) out.push({ x: m + (i % 2) * (w + g), y: m + Math.floor(i / 2) * (h + g), w, h });
  } else if (H > W) { const h = (H - 2 * m - (n - 1) * g) / n; for (let i = 0; i < n; i++) out.push({ x: m, y: m + i * (h + g), w: W - 2 * m, h }); }
  else { const w = (W - 2 * m - (n - 1) * g) / n; for (let i = 0; i < n; i++) out.push({ x: m + i * (w + g), y: m, w, h: H - 2 * m }); }
  return out;
}
function motionAt(P, t, appear, total) {
  const a = sp.motionAmt / 100;
  if (P.motion === 'none' || a <= 0) return { k: 1 };
  const u = clamp((t - appear) / Math.max(0.5, total - appear), 0, 1);
  switch (P.motion) {
    case 'in': return { k: 1 + a * u };
    case 'out': return { k: 1 + a * (1 - u) };
    case 'left': return { k: 1 + a, px: lerp(0.1, 0.9, u) };
    case 'right': return { k: 1 + a, px: lerp(0.9, 0.1, u) };
    case 'up': return { k: 1 + a, py: lerp(0.1, 0.9, u) };
    case 'down': return { k: 1 + a, py: lerp(0.9, 0.1, u) };
    default: return { k: 1 };
  }
}
const blurCanvas = document.createElement('canvas');
function drawBlurBackground(ctx, m, R) {
  const bw = Math.max(8, Math.round(R.w / 10)), bh = Math.max(8, Math.round(R.h / 10));
  blurCanvas.width = bw; blurCanvas.height = bh;
  const b = blurCanvas.getContext('2d');
  const s = Math.max(bw / m.w, bh / m.h);
  if ('filter' in b) b.filter = 'blur(3px)';
  b.drawImage(m.el, (bw - m.w * s) / 2, (bh - m.h * s) / 2, m.w * s, m.h * s);
  ctx.save();
  ctx.imageSmoothingEnabled = true; ctx.imageSmoothingQuality = 'high';
  ctx.drawImage(blurCanvas, R.x - R.w * 0.06, R.y - R.h * 0.06, R.w * 1.12, R.h * 1.12);
  ctx.fillStyle = 'rgba(0,0,0,0.32)'; ctx.fillRect(R.x, R.y, R.w, R.h);
  ctx.restore();
}
function mediaBox(i, m, R, t, TL) {
  const P = sp.panels[i];
  const mo = motionAt(P, t, TL.appear[i], TL.total);
  const base = P.fit === 'fill' ? Math.max(R.w / m.w, R.h / m.h) : Math.min(R.w / m.w, R.h / m.h);
  const s = base * P.zoom * mo.k;
  const dw = m.w * s, dh = m.h * s;
  const px = mo.px ?? P.posX, py = mo.py ?? P.posY;
  return { x: R.x + (R.w - dw) * px, y: R.y + (R.h - dh) * py, w: dw, h: dh };
}
function drawPlaceholder(ctx, R, i, minSide) {
  ctx.save();
  const lw = Math.max(2, minSide * 0.003);
  ctx.setLineDash([lw * 5, lw * 4]); ctx.lineWidth = lw; ctx.strokeStyle = 'rgba(255,255,255,0.28)';
  ctx.strokeRect(R.x + lw * 6, R.y + lw * 6, R.w - lw * 12, R.h - lw * 12);
  ctx.setLineDash([]);
  ctx.fillStyle = 'rgba(255,255,255,0.6)'; ctx.textAlign = 'center';
  const fs = Math.min(R.w, R.h) * 0.07;
  ctx.font = `600 ${fs}px Barlow, Arial, sans-serif`;
  ctx.fillText(`Panel ${i + 1}`, R.x + R.w / 2, R.y + R.h / 2 - fs * 0.3);
  ctx.font = `500 ${fs * 0.6}px Barlow, Arial, sans-serif`; ctx.fillStyle = 'rgba(255,255,255,0.45)';
  ctx.fillText('Drop a video or image', R.x + R.w / 2, R.y + R.h / 2 + fs * 0.8);
  ctx.restore();
}
function drawPanelLabels(ctx, R, i, minSide) {
  const fs = sp.labelSize * minSide / 1080;
  const margin = fs * 0.7;
  if (sp.badges) {
    const d = fs * 1.55, cx = R.x + margin + d / 2, cy = (sp.labelPos === 'top' && sp.labelStyle === 'bar' ? R.y + R.h - margin - d / 2 : R.y + margin + d / 2);
    ctx.save();
    ctx.shadowColor = 'rgba(0,0,0,0.35)'; ctx.shadowBlur = fs * 0.4;
    ctx.fillStyle = sp.labelBg; ctx.beginPath(); ctx.arc(cx, cy, d / 2, 0, Math.PI * 2); ctx.fill();
    ctx.shadowColor = 'transparent';
    ctx.fillStyle = sp.labelFg; ctx.font = `800 ${fs * 0.95}px ${LABEL_STACK}`; ctx.textAlign = 'center'; ctx.textBaseline = 'middle';
    ctx.fillText(String(i + 1), cx, cy + fs * 0.04);
    ctx.restore();
  }
  const txt = (sp.panels[i].label || '').trim();
  if (!txt) return;
  ctx.save();
  let size = fs;
  ctx.font = `700 ${size}px ${LABEL_STACK}`;
  const maxW = R.w - margin * 2 - fs * 1.2;
  let tw = ctx.measureText(txt).width;
  if (tw > maxW) { size = Math.max(fs * 0.55, size * maxW / tw); ctx.font = `700 ${size}px ${LABEL_STACK}`; tw = ctx.measureText(txt).width; }
  ctx.textAlign = 'center'; ctx.textBaseline = 'middle';
  if (sp.labelStyle === 'bar') {
    const h = size * 2.1, y = sp.labelPos === 'top' ? R.y : R.y + R.h - h;
    ctx.globalAlpha = 0.9; ctx.fillStyle = sp.labelBg; ctx.fillRect(R.x, y, R.w, h);
    ctx.globalAlpha = 1; ctx.fillStyle = sp.labelFg; ctx.fillText(txt, R.x + R.w / 2, y + h / 2 + size * 0.04);
  } else {
    const ph = size * 1.85, pw = tw + size * 1.4;
    const x = R.x + (R.w - pw) / 2, y = sp.labelPos === 'top' ? R.y + margin : R.y + R.h - margin - ph;
    ctx.shadowColor = 'rgba(0,0,0,0.35)'; ctx.shadowBlur = size * 0.5; ctx.shadowOffsetY = size * 0.1;
    ctx.fillStyle = sp.labelBg; roundRect(ctx, x, y, pw, ph, ph / 2); ctx.fill();
    ctx.shadowColor = 'transparent';
    ctx.fillStyle = sp.labelFg; ctx.fillText(txt, R.x + R.w / 2, y + ph / 2 + size * 0.04);
  }
  ctx.restore();
}
/* Before/after wipe: panel 2 shows in the part of the frame the divider has passed (from the left, or from the top) */
const wipeArea = (R, p) => (sp.wipeDir === 'tb' ? { x: R.x, y: R.y, w: R.w, h: R.h * p } : { x: R.x, y: R.y, w: R.w * p, h: R.h });
// Labels sit on their own side: panel 2's where it is revealed first (left or top), panel 1's opposite
function wipeLabelRect(R, i) {
  const first = i === 1;
  return sp.wipeDir === 'tb'
    ? { x: R.x, y: first ? R.y : R.y + R.h / 2, w: R.w, h: R.h / 2 }
    : { x: first ? R.x : R.x + R.w / 2, y: R.y, w: R.w / 2, h: R.h };
}
function drawWipeDivider(ctx, R, p, minSide) {
  const a = clamp(Math.min(p, 1 - p) / 0.03, 0, 1);  // fade out at either edge
  if (a <= 0) return;
  const lw = Math.max(3, minSide * 0.005), tb = sp.wipeDir === 'tb';
  const x = tb ? null : R.x + R.w * p, y = tb ? R.y + R.h * p : null;
  ctx.save();
  ctx.globalAlpha = a;
  ctx.shadowColor = 'rgba(0,0,0,0.45)'; ctx.shadowBlur = lw * 3;
  ctx.fillStyle = sp.wipeColor;
  if (tb) ctx.fillRect(R.x, y - lw / 2, R.w, lw); else ctx.fillRect(x - lw / 2, R.y, lw, R.h);
  if (sp.wipeHandle) {
    const r = minSide * 0.035, cx = tb ? R.x + R.w / 2 : x, cy = tb ? y : R.y + R.h / 2;
    ctx.beginPath(); ctx.arc(cx, cy, r, 0, Math.PI * 2); ctx.fill();
    ctx.shadowColor = 'transparent';
    ctx.fillStyle = '#111111';
    const s = r * 0.42;
    for (const d of [-1, 1]) {  // two small arrowheads pointing along the sweep
      ctx.beginPath();
      if (tb) { ctx.moveTo(cx - s * 0.7, cy + d * s * 0.35); ctx.lineTo(cx + s * 0.7, cy + d * s * 0.35); ctx.lineTo(cx, cy + d * s * 1.1); }
      else { ctx.moveTo(cx + d * s * 0.35, cy - s * 0.7); ctx.lineTo(cx + d * s * 0.35, cy + s * 0.7); ctx.lineTo(cx + d * s * 1.1, cy); }
      ctx.fill();
    }
  }
  ctx.restore();
}
function spDraw(ctx, W, H, t, TL, { preview = false } = {}) {
  const minSide = Math.min(W, H), vertical = H > W;
  ctx.save();
  ctx.fillStyle = sp.bg; ctx.fillRect(0, 0, W, H);
  const rects = spRects(W, H, TL.n);
  const spot = spotLevels(t, TL);
  rects.forEach((R, i) => {
    const raw = TL.entDur ? clamp((t - TL.appear[i]) / TL.entDur, 0, 1) : 1;
    if (raw <= 0) return;
    const e = EASE.smooth(raw);
    ctx.save();
    let alpha = 1;
    if (sp.entrance === 'fade') alpha = e;
    else if (sp.entrance === 'slide') {
      // In the grid, i % 2 is the column; a wipe's two panels enter as one
      if (vertical) ctx.translate((1 - e) * (i % 2 && !TL.wipe ? 1 : -1) * W * 0.7, 0); else ctx.translate(0, (1 - e) * H * 0.75);
      alpha = Math.min(1, raw * 2);
    } else if (sp.entrance === 'pop') {
      const k = lerp(0.8, 1, easeOutBack(raw)), cx = R.x + R.w / 2, cy = R.y + R.h / 2;
      ctx.translate(cx, cy); ctx.scale(k, k); ctx.translate(-cx, -cy); alpha = Math.min(1, raw * 2.5);
    } else if (sp.entrance === 'wipe') {
      ctx.beginPath();
      if (vertical) ctx.rect(R.x, R.y, R.w * e, R.h); else ctx.rect(R.x, R.y + R.h * (1 - e), R.w, R.h * e);
      ctx.clip();
    }
    ctx.globalAlpha = alpha;
    roundRect(ctx, R.x, R.y, R.w, R.h, sp.radius); ctx.clip();
    if (TL.wipe && i === 1) {
      const A = wipeArea(R, TL.wipePos(t));
      if (A.w <= 0 || A.h <= 0) { ctx.restore(); return; }
      ctx.beginPath(); ctx.rect(A.x, A.y, A.w, A.h); ctx.clip();
    }
    ctx.fillStyle = sp.panelBg; ctx.fillRect(R.x, R.y, R.w, R.h);
    const m = media[i];
    if (m && m.w && m.h) {
      if (sp.panels[i].fit === 'blur') drawBlurBackground(ctx, m, R);
      const b = mediaBox(i, m, R, t, TL);
      ctx.drawImage(m.el, b.x, b.y, b.w, b.h);
    } else if (preview) drawPlaceholder(ctx, R, i, minSide);
    if (spot && spot.dim[i] > 0) { ctx.fillStyle = `rgba(0,0,0,${0.62 * spot.dim[i]})`; ctx.fillRect(R.x, R.y, R.w, R.h); }
    drawPanelLabels(ctx, TL.wipe ? wipeLabelRect(R, i) : R, i, minSide);
    ctx.restore();
    if (spot && spot.lit[i] > 0) {
      const lw = Math.max(4, minSide * 0.007);
      ctx.save(); ctx.globalAlpha = spot.lit[i]; ctx.strokeStyle = sp.spotColor; ctx.lineWidth = lw;
      roundRect(ctx, R.x + lw / 2, R.y + lw / 2, R.w - lw, R.h - lw, Math.max(0, sp.radius - lw / 2)); ctx.stroke();
      ctx.restore();
    }
  });
  if (TL.wipe) drawWipeDivider(ctx, rects[0], TL.wipePos(t), minSide);
  if (preview && !spPlayer.playing && rects[sel]) {
    const R = rects[sel], lw = Math.max(3, minSide * 0.004);
    ctx.save(); ctx.setLineDash([lw * 4, lw * 3]); ctx.lineWidth = lw; ctx.strokeStyle = 'rgba(244,196,67,0.95)';
    ctx.strokeRect(R.x + lw / 2, R.y + lw / 2, R.w - lw, R.h - lw); ctx.restore();
  }
  ctx.restore();
}

/* ---------- preview ---------- */
function syncVideos(t, playing) {
  if (exporting || !spTL) return;
  media.forEach((m, i) => {
    if (!m || m.type !== 'video') return;
    const v = m.el;
    if (i >= spTL.n) { if (!v.paused) v.pause(); return; }
    const target = spTL.videoTime(i, t);
    const gain = clamp(gainAt(i, t, spTL), 0, 1);
    v.volume = gain; v.muted = gain === 0;
    if (playing && spTL.advancing(i, t)) {
      if (v.paused) { v.currentTime = target; v.play().catch(() => {}); }
      else if (Math.abs(v.currentTime - target) > 0.3) v.currentTime = target;
    } else {
      if (!v.paused) v.pause();
      if (!v.seeking && Math.abs(v.currentTime - target) > 0.04) v.currentTime = target;
    }
  });
}
function spPaint(t) {
  if (!spTL) return;
  const { W, H } = dims(sp.out);
  if (spCanvas.width !== W || spCanvas.height !== H) { spCanvas.width = W; spCanvas.height = H; }
  if (!exporting) syncVideos(t, spPlayer.playing);
  const ctx = spCanvas.getContext('2d');
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  spDraw(ctx, W, H, t, spTL, { preview: true });
  const g = spGuides.getContext('2d');
  const dpr = window.devicePixelRatio || 1;
  const gw = Math.round(spViewer.clientWidth * dpr), gh = Math.round(spViewer.clientHeight * dpr);
  if (spGuides.width !== gw || spGuides.height !== gh) { spGuides.width = gw; spGuides.height = gh; }
  g.clearRect(0, 0, gw, gh);
  if (spPlayer.guides) drawGuides(g, gw, gh, sp.out.aspect, gw / W * 2);
}
function spRenderTrack() {
  const tr = $('#sp-track'), TL = spTL, T = TL.total;
  const blk = (from, len, c, label, dim) => `<div class="blk${dim ? ' dim' : ''}" style="left:${from / T * 100}%;width:${Math.max(0.3, len / T * 100)}%;--c:${c}">${label ? `<span>${label}</span>` : ''}</div>`;
  let html = '';
  if (TL.entDur) for (let i = 0; i < TL.n; i++) html += blk(TL.appear[i], TL.entDur, '#8a857c', String(i + 1), true);
  if (TL.wipe) html += blk(TL.w0, TL.wipeEnd - TL.w0, '#f4c443', 'Wipe');
  if (TL.spots.length) TL.spots.forEach(s => { html += blk(s.t0, s.t1 - s.t0, sp.spotColor, `Spotlight ${s.i + 1}`); });
  else for (let i = 0; i < TL.n; i++) { const L = TL.clipLen(i); if (L) html += `<div class="blk" style="left:${TL.appear[i] / T * 100}%;width:${Math.min(L, T - TL.appear[i]) / T * 100}%;--c:#f4c443;top:auto;bottom:2px;height:3px"></div>`; }
  tr.innerHTML = html + '<div class="head"></div>';
}
function spRefresh() {
  toolSp.dataset.orient = isVertical() ? 'v' : 'h';
  spTL = spTimeline();
  const { OW, OH } = dims(sp.out);
  const n = spTL.n;
  const how = isGrid() ? 'in a 2 × 2 grid' : isWipe() ? 'as a before/after wipe' : isVertical() ? 'stacked' : 'side by side';
  $('#sp-fmt').textContent = `${OW} × ${OH} · ${n} panel${n === 1 ? '' : 's'} ${how} · ${sp.out.fps} fps`;
  const vids = media.slice(0, n).filter(m => m && m.type === 'video').length;
  const soundTxt = { none: 'Sound off', one: `Sound from panel ${sp.soundPanel + 1}`, mix: 'Sound: all panels mixed', spot: 'Sound follows the spotlight' }[sp.sound];
  $('#sp-note').textContent = vids ? soundTxt : '';
  $('#sp-dur-f').hidden = sp.autoDur;
  $('#sp-spot-f').hidden = !sp.spotlight;
  $('#sp-spotdur-f').hidden = sp.spotMatch;
  $('#sp-soundpanel-f').hidden = sp.sound !== 'one';
  $('#sp-layout-stack').textContent = isVertical() ? 'Stacked rows' : 'Side by side';
  $('#sp-layout-stack').closest('label').title = isVertical() ? '' : 'Four panels in 16:9 always use the grid';
  $('#sp-wipe-f').hidden = !isWipe();
  $('#sp-spotlight').closest('label').hidden = isWipe();
  if (isWipe()) $('#sp-spot-f').hidden = true;
  const spotOpt = $('#sp-sound option[value="spot"]'); if (spotOpt) spotOpt.disabled = !sp.spotlight;
  $('#sp-outnote').textContent = sp.out.format === 'png-seq'
    ? 'PNG frames come with the mixed sound as audio.wav in the same .zip.'
    : 'Videos are read frame by frame, so rendering runs slower than real time. Keep this tab in front.';
  spRenderTrack();
  if (spPlayer.t > spTL.total) spPlayer.seek(spTL.total); else { spPaint(spPlayer.t); spPlayer.sync(); }
  saveSoon('sp', sp);
}
function spFit() { fitViewer(spMonitor, spViewer, sp.out.aspect); spPaint(spPlayer.t); }

/* ---------- panel cards ---------- */
const MOTIONS = [['none', 'No motion'], ['in', 'Slow zoom in'], ['out', 'Slow zoom out'], ['left', 'Pan left'], ['right', 'Pan right'], ['up', 'Pan up'], ['down', 'Pan down']];
const fmtClock = s => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, '0')}`;
function drawThumb(c, m) {
  const g = c.getContext('2d'); g.fillStyle = '#000'; g.fillRect(0, 0, c.width, c.height);
  if (!m || !m.w) return;
  try { const s = Math.max(c.width / m.w, c.height / m.h); g.drawImage(m.el, (c.width - m.w * s) / 2, (c.height - m.h * s) / 2, m.w * s, m.h * s); } catch (e) { /* not ready */ }
}
function spBuildCards() {
  const host = $('#sp-panels');
  const n = spCount(), across = !isVertical() || isGrid();
  const openSet = new Set($$('.pcard[open]', host).map(d => +d.dataset.i));
  host.innerHTML = '';
  for (let i = 0; i < n; i++) {
    const P = sp.panels[i], m = media[i], isVid = m && m.type === 'video';
    const d = document.createElement('details');
    d.className = 'pcard'; d.dataset.i = i; d.dataset.sel = String(i === sel);
    if (i === sel || openSet.has(i)) d.open = true;
    const meta = !m ? 'empty' : m.sample ? 'sample' : isVid ? `video · ${fmtClock(m.duration)}${m.audioState === 'none' ? ' · no sound' : ''}` : `image · ${m.w}×${m.h}`;
    d.innerHTML = `
      <summary><span class="pnum">${i + 1}</span><canvas class="pthumb" width="88" height="60" aria-hidden="true"></canvas><span class="pname"></span><span class="pmeta">${meta}</span></summary>
      <div class="pbody">
        <div class="btn-row">
          <button class="btn small primary" data-act="pick">${m && !m.sample ? 'Replace' : 'Choose video or image'}</button>
          <button class="btn small" data-act="clear" ${m ? '' : 'disabled'}>Remove</button>
          <button class="btn small" data-act="up" ${i === 0 ? 'disabled' : ''} aria-label="Move panel ${i + 1} earlier">${across ? '←' : '↑'}</button>
          <button class="btn small" data-act="down" ${i === n - 1 ? 'disabled' : ''} aria-label="Move panel ${i + 1} later">${across ? '→' : '↓'}</button>
        </div>
        <input type="file" accept="video/*,image/*" hidden data-act="file">
        <div class="field"><span class="lbl">Fit</span>
          <div class="seg" role="radiogroup" aria-label="Fit for panel ${i + 1}">
            <label><input type="radio" name="sp-p${i}-fit" value="fill"><span>Fill &amp; crop</span></label>
            <label><input type="radio" name="sp-p${i}-fit" value="fit"><span>Show all</span></label>
            <label><input type="radio" name="sp-p${i}-fit" value="blur"><span>Show all + blur</span></label>
          </div></div>
        <div class="field"><label class="lbl" for="sp-p${i}-zoom">Zoom</label>
          <div class="range"><input type="range" id="sp-p${i}-zoom" min="1" max="4" step="0.01"><output for="sp-p${i}-zoom"></output></div></div>
        <div class="row2">
          <div class="field"><label class="lbl" for="sp-p${i}-x">Left ↔ right</label><input type="range" id="sp-p${i}-x" min="0" max="1" step="0.005"></div>
          <div class="field"><label class="lbl" for="sp-p${i}-y">Top ↕ bottom</label><input type="range" id="sp-p${i}-y" min="0" max="1" step="0.005"></div>
        </div>
        <div class="field"><label class="lbl" for="sp-p${i}-motion">Motion</label>
          <select id="sp-p${i}-motion">${MOTIONS.map(([v, l]) => `<option value="${v}">${l}</option>`).join('')}</select></div>
        <div class="field vid-only"><label class="lbl" for="sp-p${i}-trim">Start the video at</label>
          <div class="range"><input type="range" id="sp-p${i}-trim" min="0" max="${isVid ? Math.max(0, m.duration - 0.2).toFixed(2) : 0}" step="0.05"><output for="sp-p${i}-trim"></output></div></div>
        <div class="field vid-only"><label class="lbl" for="sp-p${i}-vol">Volume</label>
          <div class="range"><input type="range" id="sp-p${i}-vol" min="0" max="1" step="0.05"><output for="sp-p${i}-vol"></output></div></div>
        <label class="check vid-only"><input type="checkbox" id="sp-p${i}-loop"> Loop if it ends before the clip does</label>
        <div class="field"><label class="lbl" for="sp-p${i}-label">Label</label>
          <input type="text" id="sp-p${i}-label" placeholder="@creator, caption, or “Before”" maxlength="60"></div>
      </div>`;
    d.querySelector('.pname').textContent = m ? m.name : 'Empty';
    drawThumb(d.querySelector('.pthumb'), m);
    $$('.vid-only', d).forEach(el => { el.hidden = !isVid; });
    const changed = () => { spRefresh(); };
    const q = s => d.querySelector(s);
    const show = () => {
      $$(`input[name="sp-p${i}-fit"]`, d).forEach(r => { r.checked = r.value === P.fit; });
      q(`#sp-p${i}-zoom`).value = P.zoom; q(`output[for="sp-p${i}-zoom"]`).textContent = Math.round(P.zoom * 100) + '%';
      q(`#sp-p${i}-x`).value = P.posX; q(`#sp-p${i}-y`).value = P.posY;
      q(`#sp-p${i}-motion`).value = P.motion;
      q(`#sp-p${i}-trim`).value = P.trimIn; q(`output[for="sp-p${i}-trim"]`).textContent = fmtSec(P.trimIn);
      q(`#sp-p${i}-vol`).value = P.volume; q(`output[for="sp-p${i}-vol"]`).textContent = Math.round(P.volume * 100) + '%';
      q(`#sp-p${i}-loop`).checked = P.loop;
      if (document.activeElement !== q(`#sp-p${i}-label`)) q(`#sp-p${i}-label`).value = P.label;
    };
    d._show = show; show();
    $$(`input[name="sp-p${i}-fit"]`, d).forEach(r => r.addEventListener('change', () => { if (r.checked) { P.fit = r.value; changed(); } }));
    q(`#sp-p${i}-zoom`).addEventListener('input', e => { P.zoom = +e.target.value; show(); changed(); });
    q(`#sp-p${i}-x`).addEventListener('input', e => { P.posX = +e.target.value; changed(); });
    q(`#sp-p${i}-y`).addEventListener('input', e => { P.posY = +e.target.value; changed(); });
    q(`#sp-p${i}-motion`).addEventListener('change', e => { P.motion = e.target.value; changed(); });
    q(`#sp-p${i}-trim`).addEventListener('input', e => { P.trimIn = +e.target.value; show(); changed(); });
    q(`#sp-p${i}-vol`).addEventListener('input', e => { P.volume = +e.target.value; show(); changed(); });
    q(`#sp-p${i}-loop`).addEventListener('change', e => { P.loop = e.target.checked; changed(); });
    q(`#sp-p${i}-label`).addEventListener('input', e => { P.label = e.target.value; changed(); });
    q('[data-act="pick"]').addEventListener('click', () => q('[data-act="file"]').click());
    q('[data-act="file"]').addEventListener('change', e => { const f = e.target.files[0]; e.target.value = ''; if (f) setMedia(i, f); });
    q('[data-act="clear"]').addEventListener('click', () => { disposeMedia(media[i]); media[i] = null; spBuildCards(); spRefresh(); });
    const swap = j => {
      [sp.panels[i], sp.panels[j]] = [sp.panels[j], sp.panels[i]];
      [media[i], media[j]] = [media[j], media[i]];
      if (sp.soundPanel === i) sp.soundPanel = j; else if (sp.soundPanel === j) sp.soundPanel = i;
      sel = j; spBuildCards(); spRefresh();
    };
    q('[data-act="up"]').addEventListener('click', () => swap(i - 1));
    q('[data-act="down"]').addEventListener('click', () => swap(i + 1));
    d.addEventListener('toggle', () => { if (d.open && sel !== i) { sel = i; $$('.pcard', host).forEach(c => { c.dataset.sel = String(+c.dataset.i === sel); }); spPaint(spPlayer.t); } });
    host.appendChild(d);
  }
  const sndSel = $('#sp-soundpanel');
  sndSel.innerHTML = Array.from({ length: n }, (_, i) => `<option value="${i}">Panel ${i + 1}${media[i]?.type === 'video' ? '' : ' (no video)'}</option>`).join('');
  if (sp.soundPanel >= n) sp.soundPanel = 0;
  sndSel.value = String(sp.soundPanel);
}
function showCard(i) {
  const d = $(`.pcard[data-i="${i}"]`);
  if (d) d._show();
}

/* ---------- preview interactions: select, drop, drag to reposition, scroll to zoom ---------- */
function panelAt(e) {
  const r = spCanvas.getBoundingClientRect();
  const { W, H } = dims(sp.out);
  const x = (e.clientX - r.left) / r.width * W, y = (e.clientY - r.top) / r.height * H;
  const rects = spRects(W, H, spCount());
  let i = rects.findIndex(R => x >= R.x && x <= R.x + R.w && y >= R.y && y <= R.y + R.h);
  if (i === 0 && isWipe() && spTL) {  // both panels fill the frame: pick the one showing under the pointer
    const A = wipeArea(rects[0], spTL.wipePos(spPlayer.t));
    if (x <= A.x + A.w && y <= A.y + A.h && A.w > 0 && A.h > 0) i = 1;
  }
  return { i, x, y, R: rects[i], scale: W / r.width };
}
function selectPanel(i) {
  if (i < 0) return;
  sel = i;
  $$('#sp-panels .pcard').forEach(c => { const k = +c.dataset.i; c.dataset.sel = String(k === i); if (k === i) c.open = true; });
  spPaint(spPlayer.t);
}
let drag = null;
spCanvas.addEventListener('pointerdown', e => {
  const hit = panelAt(e); if (hit.i < 0) return;
  selectPanel(hit.i);
  const m = media[hit.i]; if (!m || spPlayer.playing) return;
  e.preventDefault();
  try { spCanvas.setPointerCapture(e.pointerId); } catch (_) { /* ignore */ }
  const b = mediaBox(hit.i, m, hit.R, spPlayer.t, spTL);
  drag = { i: hit.i, x: e.clientX, y: e.clientY, px: sp.panels[hit.i].posX, py: sp.panels[hit.i].posY, roomX: hit.R.w - b.w, roomY: hit.R.h - b.h, scale: hit.scale };
});
spCanvas.addEventListener('pointermove', e => {
  if (!drag) return;
  const P = sp.panels[drag.i];
  const dx = (e.clientX - drag.x) * drag.scale, dy = (e.clientY - drag.y) * drag.scale;
  if (Math.abs(drag.roomX) > 1) P.posX = clamp(drag.px + dx / drag.roomX, 0, 1);
  if (Math.abs(drag.roomY) > 1) P.posY = clamp(drag.py + dy / drag.roomY, 0, 1);
  showCard(drag.i); spPaint(spPlayer.t);
});
const endDrag = () => { if (drag) { drag = null; saveSoon('sp', sp); } };
spCanvas.addEventListener('pointerup', endDrag);
spCanvas.addEventListener('pointercancel', endDrag);
spCanvas.addEventListener('wheel', e => {
  const hit = panelAt(e); if (hit.i < 0 || !media[hit.i]) return;
  e.preventDefault();
  const P = sp.panels[hit.i];
  P.zoom = clamp(P.zoom * Math.exp(-e.deltaY * 0.0015), 1, 4);
  selectPanel(hit.i); showCard(hit.i); spPaint(spPlayer.t); saveSoon('sp', sp);
}, { passive: false });
spViewer.addEventListener('dragover', e => { if ([...e.dataTransfer.items].some(it => it.kind === 'file')) { e.preventDefault(); spViewer.classList.add('dropping'); } });
spViewer.addEventListener('dragleave', () => spViewer.classList.remove('dropping'));
spViewer.addEventListener('drop', e => {
  e.preventDefault(); spViewer.classList.remove('dropping');
  const hit = panelAt(e);
  setMany(e.dataTransfer.files, Math.max(0, hit.i));
});
$('#sp-pick-all').addEventListener('click', () => $('#sp-file-all').click());
$('#sp-file-all').addEventListener('change', e => { const f = [...e.target.files]; e.target.value = ''; if (f.length) setMany(f, 0); });
document.addEventListener('paste', e => {
  if (toolSp.hidden || e.target?.closest?.('input, textarea')) return;
  const item = [...(e.clipboardData?.items || [])].find(it => it.type.startsWith('image/'));
  if (item) { e.preventDefault(); setMedia(sel, item.getAsFile()); }
});

/* ---------- controls ---------- */
const spChanged = key => {
  if (key === 'out.aspect') {
    resync(sp.out);
    bindLayout.set(layoutFor());
    spBuildCards(); spFit();
  }
  if (key === 'count') {
    bindLayout.set(layoutFor());
    sel = Math.min(sel, spCount() - 1); spBuildCards();
  }
  if (key === 'layout') {
    if (sp.layout === 'grid' && sp.count !== 4) bindCount.set(4);
    else if (sp.layout === 'wipe' && sp.count !== 2) bindCount.set(2);
    else if (sp.layout === 'stack' && !isVertical() && sp.count === 4) bindCount.set(MAX_ACROSS);
    sel = Math.min(sel, spCount() - 1); spBuildCards();
  }
  if (key === 'spotlight' && sp.spotlight && sp.sound === 'one') bindSound.set('spot');
  if (key === 'spotlight' && !sp.spotlight && sp.sound === 'spot') bindSound.set('one');
  if (key === 'soundPanel') sp.soundPanel = +sp.soundPanel;
  spRefresh();
};
bindSeg('sp-orient', sp.out, 'aspect', () => spChanged('out.aspect'));
const bindCount = bindSeg('sp-count', sp, 'count', key => { sp.count = +sp.count; spChanged(key); });
const bindLayout = bindSeg('sp-layout', sp, 'layout', spChanged);
bindSeg('sp-wipemode', sp, 'wipeMode', spChanged);
bindSeg('sp-wipedir', sp, 'wipeDir', spChanged);
bindRange('sp-wipehold', sp, 'wipeHold', fmtSec, spChanged);
bindRange('sp-wipedur', sp, 'wipeDur', fmtSec, spChanged);
bindColor('sp-wipecolor', sp, 'wipeColor', spChanged);
bindCheck('sp-wipehandle', sp, 'wipeHandle', spChanged);
bindRange('sp-gap', sp, 'gap', v => v + 'px', spChanged);
bindRange('sp-margin', sp, 'margin', v => v + 'px', spChanged);
bindRange('sp-radius', sp, 'radius', v => v + 'px', spChanged);
bindColor('sp-bg', sp, 'bg', spChanged);
bindColor('sp-panelbg', sp, 'panelBg', spChanged);
bindSeg('sp-entrance', sp, 'entrance', spChanged);
bindRange('sp-delay', sp, 'delay', fmtSec, spChanged);
bindRange('sp-stagger', sp, 'stagger', fmtSec, spChanged);
bindRange('sp-entdur', sp, 'entDur', fmtSec, spChanged);
bindRange('sp-motion', sp, 'motionAmt', v => v + '%', spChanged);
bindCheck('sp-spotlight', sp, 'spotlight', spChanged);
bindCheck('sp-spotmatch', sp, 'spotMatch', spChanged);
bindRange('sp-spotdur', sp, 'spotDur', fmtSec, spChanged);
bindCheck('sp-spotplay', sp, 'spotPlay', spChanged);
bindColor('sp-spotcolor', sp, 'spotColor', spChanged);
const bindSound = bindSelect('sp-sound', sp, 'sound', spChanged);
bindSelect('sp-soundpanel', sp, 'soundPanel', spChanged);
bindCheck('sp-badges', sp, 'badges', spChanged);
bindSeg('sp-lpos', sp, 'labelPos', spChanged);
bindSeg('sp-lstyle', sp, 'labelStyle', spChanged);
bindRange('sp-lsize', sp, 'labelSize', v => v + 'px', spChanged);
bindColor('sp-lbg', sp, 'labelBg', spChanged);
bindColor('sp-lfg', sp, 'labelFg', spChanged);
bindCheck('sp-autodur', sp, 'autoDur', spChanged);
bindRange('sp-dur', sp, 'duration', fmtSec, spChanged);

const spPlayer = makePlayer('sp', {
  total: () => (spTL ? spTL.total : 1), fps: () => sp.out.fps, render: spPaint, track: $('#sp-track'),
  onPause: t => { syncVideos(t, false); spPaint(t); },
  onLoop: () => media.forEach(m => { if (m && m.type === 'video') m.el.pause(); }),
});

/* ---------- export ---------- */
function seekVideo(v, time) {
  return new Promise(res => {
    if (Math.abs(v.currentTime - time) < 0.0005 && v.readyState >= 2 && !v.seeking) { res(); return; }
    const done = () => { clearTimeout(tm); v.removeEventListener('seeked', done); res(); };
    const tm = setTimeout(done, 5000);
    v.addEventListener('seeked', done);
    v.currentTime = time;
  });
}
async function spMixAudio(TL) {
  if (sp.sound === 'none') return null;
  const users = [];
  for (let i = 0; i < TL.n; i++) {
    const m = media[i];
    if (!m || m.type !== 'video' || !m.audio || sp.panels[i].volume <= 0) continue;
    if (sp.sound === 'one' && i !== sp.soundPanel) continue;
    users.push(i);
  }
  if (!users.length) return null;
  const sr = 48000;
  const ctx = new OfflineAudioContext(2, Math.max(1, Math.ceil(TL.total * sr)), sr);
  const changes = [...new Set(TL.spots.flatMap(s => [s.t0, s.t1]))].sort((a, b) => a - b);
  for (const i of users) {
    const m = media[i], P = sp.panels[i], L = TL.clipLen(i);
    const src = ctx.createBufferSource(); src.buffer = m.audio;
    const g = ctx.createGain();
    src.connect(g); g.connect(ctx.destination);
    g.gain.setValueAtTime(gainAt(i, 0, TL), 0);
    for (const p of changes) {
      const before = gainAt(i, p - 1e-3, TL), after = gainAt(i, p + 1e-3, TL);
      if (before !== after) { g.gain.setValueAtTime(before, Math.max(0, p - 0.05)); g.gain.linearRampToValueAtTime(after, p + 0.05); }
    }
    const offset = Math.min(P.trimIn, Math.max(0, m.audio.duration - 0.01));
    if (TL.spotPlay) { const s = TL.spots[i]; src.start(s.t0, offset, Math.min(L, s.t1 - s.t0)); }
    else if (P.loop) {
      src.loop = true; src.loopStart = offset; src.loopEnd = Math.min(m.audio.duration, P.trimIn + L);
      src.start(TL.appear[i], offset);
    } else src.start(TL.appear[i], offset, L);
  }
  return ctx.startRendering();
}
async function spRenderOne() {
  spPlayer.pause();
  const TL = spTimeline();
  const { q, W, H, OW, OH } = dims(sp.out);
  const names = media.slice(0, TL.n).map(m => (m && !m.sample ? m.name.replace(/\.[^.]+$/, '') : '')).filter(Boolean);
  media.forEach(m => { if (m && m.type === 'video') { m.el.pause(); m.el.muted = true; } });
  exporting = true;
  try {
    return await runRender({
      title: `Rendering ${TL.n}-panel split`, OW, OH, fps: sp.out.fps, total: TL.total, format: sp.out.format,
      name: `split_${isGrid() ? '2x2' : isWipe() ? 'wipe' : TL.n + 'x'}_${slug(names[0] || 'clip')}_${OW}x${OH}`,
      getAudio: () => spMixAudio(TL),
      async frame(ctx, t) {
        for (let i = 0; i < TL.n; i++) {
          const m = media[i];
          if (m && m.type === 'video' && (!TL.entDur || t >= TL.appear[i])) await seekVideo(m.el, TL.videoTime(i, t));
        }
        ctx.setTransform(q, 0, 0, q, 0, 0);
        spDraw(ctx, W, H, t, TL);
      },
    });
  } finally {
    exporting = false;
    syncVideos(spPlayer.t, false); spPaint(spPlayer.t);
  }
}
async function spStill() {
  const TL = spTimeline();
  const { q, W, H, OW, OH } = dims(sp.out);
  exporting = true;
  try {
    for (let i = 0; i < TL.n; i++) { const m = media[i]; if (m && m.type === 'video') await seekVideo(m.el, TL.videoTime(i, spPlayer.t)); }
    const c = document.createElement('canvas'); c.width = OW; c.height = OH;
    const ctx = c.getContext('2d'); ctx.setTransform(q, 0, 0, q, 0, 0);
    spDraw(ctx, W, H, spPlayer.t, TL);
    await saveStill(c, `split_frame_${OW}x${OH}.png`);
  } finally { exporting = false; spPaint(spPlayer.t); }
}
buildOutputPanel('sp', sp.out, spChanged, { render: spRenderOne, still: spStill }, { aspects: ['16:9', '9:16'], both: false });
buildPresetPanel('sp', sp, SP_PRESET_KEYS, SP_BUILTINS, () => {
  sp.count = clamp(Math.round(sp.count), 1, MAX_PANELS);
  sp.layout = layoutFor();
  resync(sp); resync(sp.out);
  spBuildCards(); spFit(); spRefresh();
});
new ResizeObserver(() => { if (!toolSp.hidden) spFit(); }).observe(spMonitor);

CS.tools.sp = {
  id: 'sp',
  player: spPlayer,
  boot() {
    for (let i = 0; i < 4; i++) media[i] = makeSample(i);
    spBuildCards();
    spRefresh();
    spPlayer.seek(Math.min(spTL.total, spTL.allIn + 0.6));
  },
  show() { requestAnimationFrame(spFit); },
  hide() { spPlayer.pause(); },
  debug: { sp, spMedia: media, spPlayer, spSetMany: setMany, get spTL() { return spTL; } },
};
})();
