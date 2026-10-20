/* Clip Studio · Thumbnails tool (a still image for YouTube, Shorts, and social posts)
   Classic script (not an ES module) so index.html still works when opened straight from disk.
   Shared helpers live on window.CS; each tool registers itself in CS.tools.

   One frame, no timeline: a photo (dragged to pan, scrolled to zoom), an overlay that keeps text readable, a headline
   whose [bracketed] words are highlighted, an optional tag pill, and an optional inset picture (dragged to place).
   thLayout → thDraw; the preview, the feed-size previews, and the saved file all come from thDraw. */
(() => {
'use strict';
const {
  $, clamp, slug, clone, store, toast, roundRect, loadImageFile, fmtBytes, downloadBlob,
  resync, saveSoon, bindRange, bindColor, bindCheck, bindSeg, bindText, bindSelect, buildPresetPanel, fitViewer,
} = window.CS;

const FONTS = {
  'Montserrat': { stack: 'Montserrat, Arial, sans-serif', weight: 900 },
  'Archivo': { stack: 'Archivo, Arial, sans-serif', weight: 900 },
  'Inter': { stack: 'Inter, Arial, sans-serif', weight: 900 },
  'Oswald': { stack: 'Oswald, "Arial Narrow", sans-serif', weight: 700 },
  'Bebas Neue': { stack: '"Bebas Neue", Oswald, "Arial Narrow", sans-serif', weight: 400 },
  'Playfair Display': { stack: '"Playfair Display", Georgia, serif', weight: 900 },
};
// Saved sizes: YouTube's recommended 1280 × 720, Shorts/Reels 1080 × 1920, square posts 1080 × 1080
const SIZES = { '16:9': [1280, 720], '9:16': [1080, 1920], '1:1': [1080, 1080] };
const LIMIT = 2 * 1024 * 1024;   // YouTube's thumbnail limit
const TH_DEFAULT = {
  headline: 'Lead pipes: [10 years] to replace every line', tag: 'EXPLAINED',
  font: 'Montserrat', caps: true, size: 1, boxW: 0.6, align: 'left', valign: 'middle', tilt: 0,
  fg: '#FFFFFF', hl: '#F4C443', hlMode: 'color', hlText: '#111111',
  outline: 0, outlineColor: '#000000', shadow: 0.7,
  tagBg: '#C8102E', tagFg: '#FFFFFF',
  bg: '#1D2A44', zoom: 1, panX: 0, panY: 0, bright: 1, sat: 1.1,
  overlay: 'side', overlayColor: '#000000', overlayAmt: 0.75,
  insetShape: 'circle', insetSize: 0.55, insetX: 0.78, insetY: 0.5, insetBorder: 8, insetBorderColor: '#FFFFFF',
  out: { size: '16:9', format: 'jpeg' },
};
const TH_PRESET_KEYS = ['font', 'caps', 'size', 'boxW', 'align', 'valign', 'tilt', 'fg', 'hl', 'hlMode', 'hlText', 'outline', 'outlineColor', 'shadow', 'tagBg', 'tagFg', 'bg', 'bright', 'sat', 'overlay', 'overlayColor', 'overlayAmt', 'insetShape', 'insetSize', 'insetX', 'insetY', 'insetBorder', 'insetBorderColor', 'out'];
const TH_BUILTINS = {
  'Explainer (side fade)': { font: 'Montserrat', caps: true, size: 1, boxW: 0.6, align: 'left', valign: 'middle', tilt: 0, fg: '#FFFFFF', hl: '#F4C443', hlMode: 'color', outline: 0, shadow: 0.7, tagBg: '#C8102E', tagFg: '#FFFFFF', overlay: 'side', overlayColor: '#000000', overlayAmt: 0.75, insetShape: 'circle', insetX: 0.78, insetY: 0.5 },
  'Breaking (highlight boxes)': { font: 'Archivo', caps: true, size: 1.05, boxW: 0.9, align: 'left', valign: 'bottom', tilt: 0, fg: '#FFFFFF', hl: '#D0021B', hlMode: 'box', hlText: '#FFFFFF', outline: 0, shadow: 0.8, tagBg: '#FFFFFF', tagFg: '#D0021B', overlay: 'bottom', overlayColor: '#000000', overlayAmt: 0.85 },
  'Loud (outline and tilt)': { font: 'Bebas Neue', caps: true, size: 1.3, boxW: 0.62, align: 'left', valign: 'middle', tilt: -4, fg: '#FFFFFF', hl: '#FFE500', hlMode: 'color', outline: 7, outlineColor: '#000000', shadow: 0.6, tagBg: '#FFE500', tagFg: '#111111', overlay: 'vignette', overlayColor: '#000000', overlayAmt: 0.6, sat: 1.3 },
  'Serious (serif, darkened)': { font: 'Playfair Display', caps: false, size: 0.95, boxW: 0.8, align: 'center', valign: 'middle', tilt: 0, fg: '#FFFFFF', hl: '#F4C443', hlMode: 'color', outline: 0, shadow: 0.5, tagBg: '#F4C443', tagFg: '#111111', overlay: 'darken', overlayColor: '#0B0F17', overlayAmt: 0.6, sat: 0.8 },
  'Shorts cover (9:16)': { font: 'Montserrat', caps: true, size: 0.9, boxW: 0.88, align: 'center', valign: 'top', tilt: 0, hlMode: 'box', hl: '#F4C443', hlText: '#111111', overlay: 'bottom', overlayAmt: 0.7, insetX: 0.5, insetY: 0.72, insetSize: 0.3, out: { size: '9:16' } },
};

const th = Object.assign(clone(TH_DEFAULT), store.get('th', {}));
th.out = Object.assign({}, TH_DEFAULT.out, th.out || {});
if (!SIZES[th.out.size]) th.out.size = '16:9';
if (!FONTS[th.font]) th.font = TH_DEFAULT.font;

const toolTh = $('#tool-th');
const thCanvas = $('#th-canvas'), thViewer = $('#th-viewer'), thMonitor = $('#th-monitor');
const mctx = document.createElement('canvas').getContext('2d');
const media = { photo: null, photoName: '', photoFull: null, inset: null, insetName: '', insetFull: null };   // never stored (…Full: as it was before Remove background)
let cutBusy = false, cutWaiting = null;   // Remove background running; which picture a chosen model file is for
let thL = null;

/* ---------- layout ---------- */
/** "Lead pipes: [10 years] to go" → words with a highlight flag. Brackets can span several words. */
function parseHeadline(s) {
  const words = [];
  let on = false;
  for (const part of s.split(/(\[|\])/)) {
    if (part === '[') { on = true; continue; }
    if (part === ']') { on = false; continue; }
    for (const w of part.split(/\s+/)) if (w) words.push({ str: w, hl: on });
  }
  return words;
}
function wrap(words, font, maxW, space) {
  mctx.font = font;
  const lines = [];
  let cur = [], w = 0;
  for (const wd of words) {
    const ww = mctx.measureText(wd.str).width;
    if (cur.length && w + space + ww > maxW) { lines.push({ words: cur, w }); cur = []; w = 0; }
    wd.w = ww; cur.push(wd); w += (cur.length > 1 ? space : 0) + ww;
  }
  if (cur.length) lines.push({ words: cur, w });
  return lines;
}
function thLayout() {
  const [W, H] = SIZES[th.out.size], u = Math.min(W, H) / 720, F = FONTS[th.font];
  const mx = W * 0.05, my = H * 0.06;
  const words = parseHeadline(th.caps ? th.headline.toUpperCase() : th.headline).map(w => ({ ...w }));
  const bw = W * th.boxW - (th.align === 'center' ? 0 : mx), tag = th.tag.trim().toUpperCase();
  const ts = 30 * u, th1 = tag ? ts * 1.55 : 0, gap = tag ? 16 * u : 0;
  const maxH = H - 2 * my - th1 - gap;
  const lhK = th.font === 'Bebas Neue' || th.font === 'Oswald' ? 0.98 : th.caps ? 1.04 : 1.12;
  let fs = 104 * u * th.size, lines = [];
  for (let k = 0; k < 40; k++) {
    const font = `${F.weight} ${fs}px ${F.stack}`;
    lines = wrap(words, font, bw, fs * 0.26);
    const widest = Math.max(0, ...lines.map(l => l.w));
    if (lines.length * fs * lhK <= maxH && widest <= bw + 0.5) break;
    fs *= 0.94;
  }
  const lh = fs * lhK, blockH = th1 + gap + lines.length * lh;
  const x0 = th.align === 'center' ? (W - bw) / 2 : th.align === 'right' ? W - mx - bw : mx;
  const y0 = th.valign === 'top' ? my : th.valign === 'bottom' ? H - my - blockH : (H - blockH) / 2;
  mctx.font = `800 ${ts}px ${F.stack}`;
  const tagW = tag ? mctx.measureText(tag).width + ts * 1.3 : 0;
  return {
    W, H, u, fs, lh, lines, space: fs * 0.26, font: `${F.weight} ${fs}px ${F.stack}`, stack: F.stack,
    box: { x: x0, y: y0, w: bw, h: blockH }, tag, ts, tagH: th1, tagW, gap,
    inset: insetRect(W, H),
  };
}
function insetRect(W, H) {
  const d = th.insetSize * H, w = th.insetShape === 'circle' ? d : d * 0.8;
  return { cx: th.insetX * W, cy: th.insetY * H, w, h: d };
}
/** Where the photo lands (cover fit, then zoom and pan). */
function photoRect(img, W, H) {
  const s = Math.max(W / img.naturalWidth, H / img.naturalHeight) * th.zoom;
  const dw = img.naturalWidth * s, dh = img.naturalHeight * s;
  return { x: (W - dw) / 2 + th.panX * (dw - W) / 2, y: (H - dh) / 2 + th.panY * (dh - H) / 2, w: dw, h: dh };
}

/* ---------- drawing (preview, feed previews, and the saved file) ---------- */
function drawPhoto(ctx, L) {
  const { W, H } = L;
  ctx.fillStyle = th.bg; ctx.fillRect(0, 0, W, H);
  const img = media.photo;
  if (!img) return;
  const r = photoRect(img, W, H);
  ctx.save();
  if (th.bright !== 1 || th.sat !== 1) ctx.filter = `brightness(${th.bright}) saturate(${th.sat})`;
  ctx.drawImage(img, r.x, r.y, r.w, r.h);
  ctx.restore();
}
function hexA(hex, a) {
  const n = parseInt(hex.slice(1), 16);
  return `rgba(${n >> 16 & 255},${n >> 8 & 255},${n & 255},${a})`;
}
function drawOverlay(ctx, L) {
  const { W, H } = L, c = th.overlayColor, a = th.overlayAmt;
  if (th.overlay === 'none' || a <= 0) return;
  let g;
  if (th.overlay === 'darken') { ctx.fillStyle = hexA(c, a * 0.75); ctx.fillRect(0, 0, W, H); return; }
  if (th.overlay === 'bottom' || (th.overlay === 'side' && th.align === 'center')) {
    g = ctx.createLinearGradient(0, H * 0.3, 0, H);
    g.addColorStop(0, hexA(c, 0)); g.addColorStop(0.6, hexA(c, a * 0.7)); g.addColorStop(1, hexA(c, a));
  } else if (th.overlay === 'side') {
    const right = th.align === 'right';
    g = ctx.createLinearGradient(right ? W : 0, 0, right ? W * 0.25 : W * 0.75, 0);
    g.addColorStop(0, hexA(c, a)); g.addColorStop(0.55, hexA(c, a * 0.6)); g.addColorStop(1, hexA(c, 0));
  } else {
    g = ctx.createRadialGradient(W / 2, H / 2, Math.min(W, H) * 0.3, W / 2, H / 2, Math.hypot(W, H) * 0.55);
    g.addColorStop(0, hexA(c, 0)); g.addColorStop(1, hexA(c, a));
  }
  ctx.fillStyle = g; ctx.fillRect(0, 0, W, H);
}
function drawInset(ctx, L, sel) {
  const img = media.inset;
  if (!img) return;
  const { u } = L, r = L.inset, x = r.cx - r.w / 2, y = r.cy - r.h / 2;
  const shape = () => { if (th.insetShape === 'circle') { ctx.beginPath(); ctx.arc(r.cx, r.cy, r.w / 2, 0, Math.PI * 2); } else roundRect(ctx, x, y, r.w, r.h, 22 * u); };
  ctx.save();
  ctx.shadowColor = 'rgba(0,0,0,0.45)'; ctx.shadowBlur = 40 * u; ctx.shadowOffsetY = 10 * u;
  shape(); ctx.fillStyle = th.insetBorder > 0 ? th.insetBorderColor : '#000'; ctx.fill();
  ctx.restore();
  ctx.save();
  shape(); ctx.clip();
  const s = Math.max(r.w / img.naturalWidth, r.h / img.naturalHeight);
  ctx.drawImage(img, r.cx - img.naturalWidth * s / 2, r.cy - img.naturalHeight * s / 2, img.naturalWidth * s, img.naturalHeight * s);
  ctx.restore();
  if (th.insetBorder > 0) { ctx.save(); shape(); ctx.lineWidth = th.insetBorder * u; ctx.strokeStyle = th.insetBorderColor; ctx.stroke(); ctx.restore(); }
  if (sel) { ctx.save(); ctx.setLineDash([10 * u, 8 * u]); ctx.lineWidth = 2 * u; ctx.strokeStyle = 'rgba(244,196,67,0.9)'; ctx.strokeRect(x - 6 * u, y - 6 * u, r.w + 12 * u, r.h + 12 * u); ctx.restore(); }
}
function drawText(ctx, L) {
  const { u, box, lh, fs, space } = L;
  ctx.save();
  if (th.tilt) { const cx = box.x + box.w / 2, cy = box.y + box.h / 2; ctx.translate(cx, cy); ctx.rotate(th.tilt * Math.PI / 180); ctx.translate(-cx, -cy); }
  const lineX = w => (th.align === 'center' ? box.x + (box.w - w) / 2 : th.align === 'right' ? box.x + box.w - w : box.x);
  const shadowOn = () => { if (th.shadow > 0) { ctx.shadowColor = `rgba(0,0,0,${0.85 * th.shadow})`; ctx.shadowBlur = 34 * u * th.shadow; ctx.shadowOffsetY = 8 * u * th.shadow; } };
  const shadowOff = () => { ctx.shadowColor = 'transparent'; ctx.shadowBlur = 0; ctx.shadowOffsetY = 0; };
  if (L.tag) {
    const x = lineX(L.tagW);
    ctx.save(); shadowOn();
    ctx.fillStyle = th.tagBg; roundRect(ctx, x, box.y, L.tagW, L.tagH, L.tagH / 2); ctx.fill();
    ctx.restore();
    ctx.font = `800 ${L.ts}px ${L.stack}`; ctx.fillStyle = th.tagFg; ctx.textBaseline = 'middle'; ctx.textAlign = 'center';
    ctx.fillText(L.tag, x + L.tagW / 2, box.y + L.tagH / 2 + L.ts * 0.04);
  }
  ctx.font = L.font; ctx.textBaseline = 'alphabetic'; ctx.textAlign = 'left'; ctx.lineJoin = 'round';
  const top = box.y + L.tagH + L.gap;
  // Highlight boxes first, one per run of highlighted words on a line, so neighbouring words share a box
  if (th.hlMode === 'box') {
    ctx.save(); shadowOn(); ctx.fillStyle = th.hl;
    L.lines.forEach((ln, li) => {
      let x = lineX(ln.w), run = null;
      const flush = () => { if (run) { const p = fs * 0.16; roundRect(ctx, run.a - p, top + li * lh + fs * 0.02, run.b - run.a + 2 * p, lh * 0.98, 6 * u); ctx.fill(); run = null; } };
      ln.words.forEach((wd, wi) => {
        if (wd.hl) { if (run) run.b = x + wd.w; else run = { a: x, b: x + wd.w }; } else flush();
        x += wd.w + (wi < ln.words.length - 1 ? space : 0);
      });
      flush();
    });
    ctx.restore();
  }
  for (const pass of ['stroke', 'fill']) {
    if (pass === 'stroke' && !th.outline) continue;
    ctx.save();
    if (pass === (th.outline ? 'stroke' : 'fill') && th.hlMode !== 'box') shadowOn();
    if (pass === 'stroke') { ctx.lineWidth = th.outline * 2 * u; ctx.strokeStyle = th.outlineColor; }
    L.lines.forEach((ln, li) => {
      let x = lineX(ln.w);
      const base = top + li * lh + fs * 0.86;
      for (const wd of ln.words) {
        if (pass === 'stroke') ctx.strokeText(wd.str, x, base);
        else { ctx.fillStyle = wd.hl ? (th.hlMode === 'box' ? th.hlText : th.hl) : th.fg; ctx.fillText(wd.str, x, base); }
        x += wd.w + space;
      }
    });
    ctx.restore();
    shadowOff();
  }
  ctx.restore();
}
function thDraw(ctx, L, opts = {}) {
  ctx.save();
  drawPhoto(ctx, L);
  drawOverlay(ctx, L);
  drawInset(ctx, L, opts.selInset);
  drawText(ctx, L);
  ctx.restore();
}

/* ---------- preview ---------- */
let selInset = false;
function thPaint() {
  if (!thL) return;
  const { W, H } = thL;
  if (thCanvas.width !== W || thCanvas.height !== H) { thCanvas.width = W; thCanvas.height = H; }
  const ctx = thCanvas.getContext('2d');
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  thDraw(ctx, thL, { selInset });
  paintFeed();
}
/** The thumbnail at the sizes people see it: a feed card and a small sidebar card, with a duration badge on 16:9. */
function paintFeed() {
  const clean = document.createElement('canvas'); clean.width = thL.W; clean.height = thL.H;
  thDraw(clean.getContext('2d'), thL);
  for (const [id, base] of [['th-feed-lg', 300], ['th-feed-sm', 150]]) {
    const c = $('#' + id), dpr = window.devicePixelRatio || 1;
    const vert = thL.H > thL.W, cw = vert ? base * 0.5 : thL.W === thL.H ? base * 0.62 : base;
    const ch = cw * thL.H / thL.W;
    c.style.width = cw + 'px'; c.style.height = ch + 'px';
    c.width = Math.round(cw * dpr); c.height = Math.round(ch * dpr);
    const g = c.getContext('2d');
    g.imageSmoothingQuality = 'high';
    g.drawImage(clean, 0, 0, c.width, c.height);
    if (th.out.size === '16:9') {
      const s = c.width / 300, fsz = Math.max(9 * dpr, 12 * s), bw = fsz * 3, bh = fsz * 1.45;
      g.fillStyle = 'rgba(0,0,0,0.8)'; roundRect(g, c.width - bw - 6 * s, c.height - bh - 6 * s, bw, bh, 3 * s); g.fill();
      g.fillStyle = '#fff'; g.font = `600 ${fsz}px Barlow, Arial, sans-serif`; g.textAlign = 'center'; g.textBaseline = 'middle';
      g.fillText('12:34', c.width - bw / 2 - 6 * s, c.height - bh / 2 - 6 * s);
    }
  }
}
function thRefresh() {
  thL = thLayout();
  const [W, H] = SIZES[th.out.size];
  $('#th-fmt').textContent = `${W} × ${H} · ${th.out.format === 'jpeg' ? 'JPEG' : 'PNG'}`;
  $('#th-empty').hidden = !!media.photo;
  toolTh.dataset.inset = media.inset ? 'on' : 'off';
  toolTh.dataset.hl = th.hlMode;
  thPaint();
  saveSoon('th', th);
}
function thFit() { fitViewer(thMonitor, thViewer, th.out.size); thPaint(); }
async function thEnsureFonts() {
  const F = FONTS[th.font];
  try { await Promise.all([document.fonts.load(`${F.weight} 60px ${F.stack}`, 'AaGg09'), document.fonts.load(`800 30px ${F.stack}`, 'AaGg')]); } catch (e) { /* fall back silently */ }
}

/* ---------- controls ---------- */
const binds = {};
const thChanged = key => {
  if (key === 'out.size') thFit();
  if (key === 'font') thEnsureFonts().then(thRefresh);
  thRefresh();
};
bindText('th-headline', th, 'headline', thChanged);
bindText('th-tag', th, 'tag', thChanged);
bindCheck('th-caps', th, 'caps', thChanged);
bindSelect('th-font', th, 'font', thChanged);
bindRange('th-size', th, 'size', v => `${Math.round(v * 100)}%`, thChanged);
bindRange('th-boxw', th, 'boxW', v => `${Math.round(v * 100)}%`, thChanged);
bindSeg('th-align', th, 'align', thChanged);
bindSeg('th-valign', th, 'valign', thChanged);
bindRange('th-tilt', th, 'tilt', v => `${v}°`, thChanged);
bindColor('th-fg', th, 'fg', thChanged);
bindColor('th-hl', th, 'hl', thChanged);
bindSeg('th-hlmode', th, 'hlMode', thChanged);
bindColor('th-hltext', th, 'hlText', thChanged);
bindRange('th-outline', th, 'outline', v => `${v}px`, thChanged);
bindColor('th-outlinecolor', th, 'outlineColor', thChanged);
bindRange('th-shadow', th, 'shadow', v => `${Math.round(v * 100)}%`, thChanged);
bindColor('th-tagbg', th, 'tagBg', thChanged);
bindColor('th-tagfg', th, 'tagFg', thChanged);
bindColor('th-bg', th, 'bg', thChanged);
binds.zoom = bindRange('th-zoom', th, 'zoom', v => `${Math.round(v * 100)}%`, thChanged);
bindRange('th-bright', th, 'bright', v => `${Math.round(v * 100)}%`, thChanged);
bindRange('th-sat', th, 'sat', v => `${Math.round(v * 100)}%`, thChanged);
bindSeg('th-overlay', th, 'overlay', thChanged);
bindColor('th-overlaycolor', th, 'overlayColor', thChanged);
bindRange('th-overlayamt', th, 'overlayAmt', v => `${Math.round(v * 100)}%`, thChanged);
bindSeg('th-insetshape', th, 'insetShape', thChanged);
binds.insetSize = bindRange('th-insetsize', th, 'insetSize', v => `${Math.round(v * 100)}%`, thChanged);
bindRange('th-insetborder', th, 'insetBorder', v => `${v}px`, thChanged);
bindColor('th-insetbordercolor', th, 'insetBorderColor', thChanged);
bindSeg('th-size-out', th.out, 'size', () => thChanged('out.size'));
bindSeg('th-format', th.out, 'format', () => thChanged('out.format'));

async function setImage(which, f) {
  try {
    media[which] = await loadImageFile(f); media[which + 'Name'] = f.name; media[which + 'Full'] = null; cutNote(which, '');
    if (which === 'photo') { th.panX = 0; th.panY = 0; binds.zoom.set(1); }
    mediaUI(); thRefresh();
    return true;
  } catch (err) { toast('That file couldn’t be opened as an image. Try a JPEG, PNG, or WebP.'); return false; }
}
// the app's own stills (ClipCut: a frame of its video) come in as the photo
window.CS.hosted.onPhoto(f => setImage('photo', f));

/* A frame of a video as the photo: the video plays in a dialog, the slider picks the frame */
const FR = { url: null, name: '', fps: 30 };
const frVideo = $('#frame-video');
function fmtAt(t) { const m = Math.floor(t / 60); return `${m}:${(t - m * 60).toFixed(2).padStart(5, '0')}`; }
async function openFramePicker(f) {
  if (FR.url) URL.revokeObjectURL(FR.url);
  FR.url = URL.createObjectURL(f);
  FR.name = f.name.replace(/\.[^.]+$/, '');
  frVideo.src = FR.url;
  try {
    await new Promise((res, rej) => { frVideo.onloadeddata = res; frVideo.onerror = () => rej(new Error('bad')); });
  } catch (_) { toast('That video couldn’t be opened here. Try an MP4 (H.264) or WebM file.'); return; }
  const r = $('#frame-time');
  r.max = String(Math.max(0, frVideo.duration - 0.001));
  r.value = String(Math.min(frVideo.duration / 2, 1));
  frVideo.currentTime = +r.value;
  $('#frame-at').textContent = fmtAt(+r.value);
  $('#frame-name').textContent = f.name;
  $('#frame-dlg').showModal();
}
function seekFrame(t) {
  const r = $('#frame-time');
  t = Math.min(Math.max(0, t), +r.max);
  r.value = String(t);
  frVideo.currentTime = t;
  $('#frame-at').textContent = fmtAt(t);
}
$('#frame-time').addEventListener('input', e => seekFrame(+e.target.value));
$('#frame-prev').addEventListener('click', () => seekFrame(frVideo.currentTime - 1 / FR.fps));
$('#frame-next').addEventListener('click', () => seekFrame(frVideo.currentTime + 1 / FR.fps));
$('#frame-cancel').addEventListener('click', () => $('#frame-dlg').close());
$('#frame-dlg').addEventListener('close', () => { frVideo.removeAttribute('src'); frVideo.load(); if (FR.url) { URL.revokeObjectURL(FR.url); FR.url = null; } });
$('#frame-use').addEventListener('click', async () => {
  const c = document.createElement('canvas');
  c.width = frVideo.videoWidth; c.height = frVideo.videoHeight;
  if (!c.width || !c.height) { toast('That frame couldn’t be read. Move the slider a little and try again.'); return; }
  c.getContext('2d').drawImage(frVideo, 0, 0);
  const blob = await new Promise(res => c.toBlob(res, 'image/png'));
  if (!blob) { toast('That frame couldn’t be read. Move the slider a little and try again.'); return; }
  const name = `${FR.name} ${fmtAt(frVideo.currentTime).replace(':', '.')}.png`;
  $('#frame-dlg').close();
  setImage('photo', new File([blob], name, { type: 'image/png' }));
});
$('#th-photo-video').addEventListener('click', () => $('#th-video-file').click());
$('#th-video-file').addEventListener('change', e => { const f = e.target.files[0]; e.target.value = ''; if (f) openFramePicker(f); });
for (const which of ['photo', 'inset']) {
  $(`#th-${which}-file`).addEventListener('change', e => { const f = e.target.files[0]; e.target.value = ''; if (f) setImage(which, f); });
}
function pickImage(which) { $(`#th-${which}-file`).click(); }
// Dropping an image on the preview makes it the photo
thViewer.addEventListener('dragover', e => { if ([...e.dataTransfer.items].some(i => i.kind === 'file')) { e.preventDefault(); thViewer.classList.add('dropping'); } });
thViewer.addEventListener('dragleave', () => thViewer.classList.remove('dropping'));
thViewer.addEventListener('drop', e => {
  e.preventDefault(); thViewer.classList.remove('dropping');
  const files = [...e.dataTransfer.files];
  const f = files.find(x => x.type.startsWith('image/'));
  const v = files.find(x => x.type.startsWith('video/'));
  if (f) setImage('photo', f); else if (v) openFramePicker(v); else toast('Drop an image file (JPEG, PNG, or WebP), or a video to pick a frame from.');
});
function mediaUI() {
  for (const which of ['photo', 'inset']) {
    $(`#th-${which}-name`).textContent = media[which + 'Name'];
    $(`#th-${which}-clear`).hidden = !media[which];
    document.querySelector(`.th-${which}-has`).hidden = !media[which];
    const b = $(`#th-${which}-cut`);
    b.textContent = media[which + 'Full'] ? 'Put background back' : 'Remove background';
    b.disabled = cutBusy;
  }
}

/* Remove background: the photo or inset through the cut-out model (CS.cutout); the picture as it was
   is kept, so the background can come back */
function cutNote(which, text, chooseFile = false) {
  const n = $(`#th-${which}-cutnote`);
  n.hidden = !text;
  n.textContent = text || '';
  if (chooseFile) {
    const b = document.createElement('button');
    b.className = 'btn small'; b.type = 'button'; b.textContent = 'Choose model file';
    b.addEventListener('click', () => { cutWaiting = which; $('#th-model-file').click(); });
    n.append(' ', b);
  }
}
async function toggleCut(which) {
  if (cutBusy || !media[which]) return;
  if (media[which + 'Full']) {
    media[which] = media[which + 'Full']; media[which + 'Full'] = null;
    cutNote(which, ''); mediaUI(); thRefresh();
    return;
  }
  cutBusy = true; mediaUI();
  const src = media[which];
  const t0 = performance.now();
  try {
    cutNote(which, 'Getting the background model ready…');
    const img = await window.CS.cutout(src, (done, total, phase) => {
      if (phase === 'start') cutNote(which, 'Removing the background… (a few seconds on a graphics card, up to half a minute without one)');
      else cutNote(which, `Downloading the background model, once: ${Math.round(done / 1e6)} of ${Math.round(total / 1e6)} MB`);
    });
    if (media[which] !== src) return; // another picture was chosen meanwhile
    media[which + 'Full'] = src;
    media[which] = img;
    cutNote(which, `Background removed in ${((performance.now() - t0) / 1000).toFixed(1)} s. Hair and soft edges may need a plainer background colour behind them.`);
    thRefresh();
  } catch (e) {
    cutNote(which, e.message || String(e), !!e.needsFile);
  } finally { cutBusy = false; mediaUI(); }
}
$('#th-photo-cut').addEventListener('click', () => toggleCut('photo'));
$('#th-inset-cut').addEventListener('click', () => toggleCut('inset'));
$('#th-model-file').addEventListener('change', async e => {
  const f = e.target.files[0]; e.target.value = '';
  const which = cutWaiting || 'photo';
  cutWaiting = null;
  if (!f) return;
  try { await window.CS.cutout.useFile(f); cutNote(which, 'Model ready.'); toggleCut(which); } catch (err) { cutNote(which, err.message, true); }
});
$('#th-photo-pick').addEventListener('click', () => pickImage('photo'));
$('#th-inset-pick').addEventListener('click', () => pickImage('inset'));
$('#th-photo-clear').addEventListener('click', () => { media.photo = null; media.photoFull = null; media.photoName = ''; cutNote('photo', ''); mediaUI(); thRefresh(); });
$('#th-inset-clear').addEventListener('click', () => { media.inset = null; media.insetFull = null; media.insetName = ''; selInset = false; cutNote('inset', ''); mediaUI(); thRefresh(); });
$('#th-recenter').addEventListener('click', () => { th.panX = 0; th.panY = 0; binds.zoom.set(1); thRefresh(); });

// Drag on the preview: the inset picture moves when you grab it; anywhere else pans the photo. Scroll zooms the photo.
const toFrame = e => { const r = thCanvas.getBoundingClientRect(); return { x: (e.clientX - r.left) * thL.W / r.width, y: (e.clientY - r.top) * thL.H / r.height, k: thL.W / r.width }; };
function overInset(p) {
  if (!media.inset) return false;
  const r = thL.inset;
  if (th.insetShape === 'circle') return Math.hypot(p.x - r.cx, p.y - r.cy) <= r.w / 2;
  return Math.abs(p.x - r.cx) <= r.w / 2 && Math.abs(p.y - r.cy) <= r.h / 2;
}
let drag = null;
thCanvas.addEventListener('pointerdown', e => {
  const p = toFrame(e);
  if (overInset(p)) drag = { kind: 'inset', p, x: th.insetX, y: th.insetY };
  else if (media.photo) drag = { kind: 'photo', p, x: th.panX, y: th.panY, r: photoRect(media.photo, thL.W, thL.H) };
  else return;
  selInset = drag.kind === 'inset';
  thCanvas.setPointerCapture(e.pointerId);
  thPaint();
});
thCanvas.addEventListener('pointermove', e => {
  if (!drag) { if (thL) thCanvas.style.cursor = overInset(toFrame(e)) ? 'move' : media.photo ? 'grab' : ''; return; }
  const p = toFrame(e), dx = p.x - drag.p.x, dy = p.y - drag.p.y;
  if (drag.kind === 'inset') { th.insetX = clamp(drag.x + dx / thL.W, 0, 1); th.insetY = clamp(drag.y + dy / thL.H, 0, 1); }
  else {
    const ex = (drag.r.w - thL.W) / 2, ey = (drag.r.h - thL.H) / 2;
    if (ex > 0.5) th.panX = clamp(drag.x + dx / ex, -1, 1);
    if (ey > 0.5) th.panY = clamp(drag.y + dy / ey, -1, 1);
  }
  thRefresh();
});
const endDrag = () => { if (!drag) return; drag = null; selInset = false; thPaint(); };
thCanvas.addEventListener('pointerup', endDrag);
thCanvas.addEventListener('pointercancel', endDrag);
thCanvas.addEventListener('wheel', e => {
  if (!media.photo && !overInset(toFrame(e))) return;
  e.preventDefault();
  if (overInset(toFrame(e))) binds.insetSize.set(clamp(+(th.insetSize * Math.exp(-e.deltaY * 0.0015)).toFixed(3), 0.12, 0.9));
  else binds.zoom.set(clamp(+(th.zoom * Math.exp(-e.deltaY * 0.0015)).toFixed(3), 1, 4));
  thRefresh();
}, { passive: false });

/* ---------- saving ---------- */
const toBlob = (c, type, q) => new Promise(r => c.toBlob(r, type, q));
/** The file to save: PNG, or JPEG at the best quality that stays under YouTube's 2 MB limit. */
async function thEncode() {
  await thEnsureFonts(); thRefresh();
  const c = document.createElement('canvas'); c.width = thL.W; c.height = thL.H;
  thDraw(c.getContext('2d'), thL);
  if (th.out.format === 'png') return { blob: await toBlob(c, 'image/png'), ext: 'png' };
  let blob = null;
  for (const q of [0.92, 0.85, 0.78, 0.7, 0.6, 0.5]) { blob = await toBlob(c, 'image/jpeg', q); if (blob.size <= LIMIT) break; }
  return { blob, ext: 'jpg' };
}
async function thSave() {
  const { blob, ext } = await thEncode();
  const [W, H] = SIZES[th.out.size];
  const name = `thumbnail_${slug(th.headline.replace(/[[\]]/g, ''))}_${W}x${H}.${ext}`;
  downloadBlob(blob, name);
  const st = $('#th-save-status');
  st.textContent = blob.size > LIMIT
    ? `Saved ${name} (${fmtBytes(blob.size)}). That’s over YouTube’s 2 MB limit: choose JPEG to save a smaller file.`
    : `Saved ${name} (${fmtBytes(blob.size)}).`;
  st.className = blob.size > LIMIT ? 'status err' : 'status ok';
  return { name, size: blob.size };
}
$('#th-save').addEventListener('click', () => thSave());
buildPresetPanel('th', th, TH_PRESET_KEYS, TH_BUILTINS, () => { resync(th); resync(th.out); thEnsureFonts().then(thRefresh); thFit(); thRefresh(); });
new ResizeObserver(() => { if (!toolTh.hidden) thFit(); }).observe(thMonitor);

CS.tools.th = {
  id: 'th',
  player: null,   // a still image: nothing to play
  boot() { mediaUI(); thRefresh(); thEnsureFonts().then(thRefresh); },
  show() { requestAnimationFrame(thFit); },
  hide() { drag = null; },
  debug: { th, thMedia: media, get thL() { return thL; }, thParseHeadline: parseHeadline, thEncode },
};
})();
