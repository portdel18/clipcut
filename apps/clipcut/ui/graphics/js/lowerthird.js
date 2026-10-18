/* Clip Studio · Lower Thirds tool (name bars, breaking straps, location tags)
   Classic script (not an ES module) so index.html still works when opened straight from disk.
   Shared helpers live on window.CS; each tool registers itself in CS.tools.

   A lower third is a few pieces (boxes, a rule, lines of text) laid out by ltLayout for the chosen template. ltTimeline
   gives the in, hold, and out times; each piece animates in a little after the one before it and leaves in reverse
   order. ltDraw is shared by the preview and the export. The background is transparent for PNG frames, or green
   screen or a color for video, so the result can sit over footage in any editor. */
(() => {
'use strict';
const {
  $, clamp, lerp, EASE, easeOutBack, fmtSec, slug, clone, store, toast, roundRect, loadImageFile,
  FRAMES, dims, resync, saveSoon, bindRange, bindColor, bindCheck, bindSeg, bindText, bindSelect,
  buildOutputPanel, buildPresetPanel, makePlayer, fitViewer, drawGuides, runRender, renderBoth, saveStill, hosted,
} = window.CS;

const FONTS = {
  'Libre Franklin': '"Libre Franklin", Arial, sans-serif',
  'Inter': 'Inter, Arial, sans-serif',
  'Roboto Condensed': '"Roboto Condensed", "Arial Narrow", sans-serif',
  'Oswald': 'Oswald, "Arial Narrow", sans-serif',
  'Montserrat': 'Montserrat, Arial, sans-serif',
  'Source Serif 4': '"Source Serif 4", Georgia, serif',
};
const GREEN = '#00B140';
const STYLES = ['bar', 'breaking', 'location', 'minimal'];
const LT_DEFAULT = {
  style: 'bar', line1: 'Jordan Ellis', line2: 'City Council member, District 4', tag: 'LIVE',
  caps1: false, caps2: false,
  font: 'Libre Franklin', size: 1, pos: 'left', lift: 0.12, radius: 0, shadow: true,
  bar: '#FFFFFF', text1: '#111418', band: '#1D2A44', text2: '#FFFFFF', accent: '#C8102E', tagText: '#FFFFFF',
  bgMode: 'transparent', bgColor: '#000000',
  anim: 'wipe', delay: 0.3, inDur: 0.7, hold: 4, outOn: true, outDur: 0.5,
  out: { aspect: '16:9', quality: '1080', fps: 30, format: 'png-seq' },
};
const LT_PRESET_KEYS = ['style', 'caps1', 'caps2', 'font', 'size', 'pos', 'lift', 'radius', 'shadow', 'bar', 'text1', 'band', 'text2', 'accent', 'tagText', 'bgMode', 'bgColor', 'anim', 'delay', 'inDur', 'hold', 'outOn', 'outDur', 'out'];
const LT_BUILTINS = {
  'Network name bar': { style: 'bar', caps1: false, caps2: false, font: 'Libre Franklin', pos: 'left', radius: 0, shadow: true, bar: '#FFFFFF', text1: '#111418', band: '#1D2A44', text2: '#FFFFFF', accent: '#C8102E', tagText: '#FFFFFF', anim: 'wipe', inDur: 0.7, outDur: 0.5 },
  'Breaking strap': { style: 'breaking', caps1: true, caps2: false, font: 'Roboto Condensed', radius: 0, shadow: true, bar: '#FFFFFF', text1: '#0B0B0C', band: '#111111', text2: '#FFFFFF', accent: '#D0021B', tagText: '#FFFFFF', anim: 'slide', inDur: 0.6, outDur: 0.4 },
  'Location tag': { style: 'location', caps1: true, caps2: false, font: 'Montserrat', pos: 'left', radius: 6, shadow: true, band: '#0F1216', text2: '#FFFFFF', accent: '#F4C443', tagText: '#111111', anim: 'pop', inDur: 0.6, outDur: 0.4 },
  'Clean minimal': { style: 'minimal', caps1: false, caps2: false, font: 'Inter', pos: 'left', shadow: true, text2: '#FFFFFF', accent: '#F4C443', anim: 'fade', inDur: 0.8, outDur: 0.6 },
  'Vertical name bar (9:16)': { style: 'bar', font: 'Libre Franklin', pos: 'center', size: 1.15, lift: 0.24, radius: 4, anim: 'pop', out: { aspect: '9:16' } },
};

const lt = Object.assign(clone(LT_DEFAULT), store.get('lt', {}));
lt.out = Object.assign({}, LT_DEFAULT.out, lt.out || {});
if (!FRAMES[lt.out.aspect]) lt.out.aspect = '16:9';
if (!FONTS[lt.font]) lt.font = LT_DEFAULT.font;
if (!STYLES.includes(lt.style)) lt.style = 'bar';

const toolLt = $('#tool-lt');
const ltCanvas = $('#lt-canvas'), ltGuides = $('#lt-guides'), ltViewer = $('#lt-viewer'), ltMonitor = $('#lt-monitor');
const mctx = document.createElement('canvas').getContext('2d');
let ltL = null, ltTL = null, bgImg = null;

/* ---------- layout ---------- */
const capsOf = (s, on) => (on ? s.toUpperCase() : s);
function textW(str, font) { mctx.font = font; return mctx.measureText(str).width; }
/** Pieces in frame pixels: { x, y, w, h, fill?, rule?, pin?, texts: [{ str, font, color, alpha, base, align, inset }] }.
    Text sits at the piece's left + inset (align left), right − inset (right), or center. */
function ltLayout() {
  const [W, H] = FRAMES[lt.out.aspect];
  const minSide = Math.min(W, H), stack = FONTS[lt.font];
  const mx = W * 0.06, maxW = W - 2 * mx, bottom = H * (1 - lt.lift);
  const l1 = capsOf(lt.line1.trim(), lt.caps1), l2 = capsOf(lt.line2.trim(), lt.caps2), tag = lt.tag.trim().toUpperCase();
  const build = u => ({ bar: buildBar, breaking: buildBreaking, location: buildLocation, minimal: buildMinimal })[lt.style](u, { W, H, mx, maxW, bottom, stack, l1, l2, tag });
  let u = minSide / 1080 * lt.size, L = build(u);
  // Too wide for the frame: shrink everything to fit (the breaking strap fits its own text instead)
  if (L.groupW > maxW + 0.5) { u *= maxW / L.groupW; L = build(u); }
  return Object.assign(L, { W, H, u, mx, minSide, empty: !l1 && !l2 });
}
/** Places a group laid out from x = 0 at the left, center, or right of the frame. Right mirrors it, so the accent
    and text hug the right edge. */
function place(pieces, groupW, c, pos = lt.pos) {
  const x0 = pos === 'right' ? c.W - c.mx - groupW : pos === 'center' ? (c.W - groupW) / 2 : c.mx;
  for (const p of pieces) {
    if (pos === 'right') {
      p.x = groupW - p.x - p.w;
      for (const tx of p.texts) tx.align = tx.align === 'left' ? 'right' : tx.align === 'right' ? 'left' : tx.align;
    }
    p.x += x0;
  }
  return { pieces, groupW, side: pos };
}
function buildBar(u, c) {
  const pad = 28 * u, s1 = 56 * u, s2 = 32 * u, aw = 14 * u;
  const f1 = `700 ${s1}px ${c.stack}`, f2 = `400 ${s2}px ${c.stack}`, ft = `700 ${s2 * 0.78}px ${c.stack}`;
  const h1 = s1 * 1.55, h2 = c.l2 ? s2 * 1.75 : 0;
  const mainW = Math.max(360 * u, (c.l1 ? textW(c.l1, f1) : 0) + 2 * pad);
  const bandW = c.l2 ? textW(c.l2, f2) + 2 * pad : 0;
  const top = c.bottom - h1 - h2, pieces = [];
  pieces.push({ x: 0, y: top, w: aw, h: h1 + h2, fill: lt.accent, texts: [] });
  if (c.tag) {
    const th = s2 * 1.2, tw = textW(c.tag, ft) + pad * 0.9;
    pieces.push({ x: 0, y: top - th, w: tw, h: th, fill: lt.accent, texts: [{ str: c.tag, font: ft, color: lt.tagText, base: top - th * 0.29, align: 'left', inset: pad * 0.45 }] });
  }
  pieces.push({ x: aw, y: top, w: mainW, h: h1, fill: lt.bar, texts: [{ str: c.l1, font: f1, color: lt.text1, base: top + h1 * 0.5 + s1 * 0.35, align: 'left', inset: pad }] });
  if (c.l2) pieces.push({ x: aw, y: top + h1, w: bandW, h: h2, fill: lt.band, texts: [{ str: c.l2, font: f2, color: lt.text2, base: top + h1 + h2 * 0.5 + s2 * 0.35, align: 'left', inset: pad }] });
  return place(pieces, aw + Math.max(mainW, bandW), c);
}
function buildBreaking(u, c) {
  const pad = 26 * u, h1 = 88 * u, st = 42 * u, s2 = 30 * u, h2 = c.l2 ? s2 * 1.75 : 0;
  const ft = `700 ${st}px ${c.stack}`, f2 = `400 ${s2}px ${c.stack}`;
  const gw = c.maxW, top = c.bottom - h1 - h2, pieces = [];
  const tagW = c.tag ? textW(c.tag, ft) + 2 * pad : 0;
  if (c.tag) pieces.push({ x: 0, y: top, w: tagW, h: h1, fill: lt.accent, texts: [{ str: c.tag, font: ft, color: lt.tagText, base: top + h1 * 0.5 + st * 0.36, align: 'left', inset: pad }] });
  // The headline shrinks to fit its bar rather than shrinking the whole strap
  const room = gw - tagW - 2 * pad;
  let s1 = 48 * u; const w1 = c.l1 ? textW(c.l1, `700 ${s1}px ${c.stack}`) : 0;
  if (w1 > room) s1 *= room / w1;
  const f1 = `700 ${s1}px ${c.stack}`;
  pieces.push({ x: tagW, y: top, w: gw - tagW, h: h1, fill: lt.bar, texts: [{ str: c.l1, font: f1, color: lt.text1, base: top + h1 * 0.5 + s1 * 0.36, align: 'left', inset: pad }] });
  if (c.l2) {
    let fs2 = s2; const w2 = textW(c.l2, f2); if (w2 > gw - 2 * pad) fs2 *= (gw - 2 * pad) / w2;
    pieces.push({ x: 0, y: top + h1, w: gw, h: h2, fill: lt.band, texts: [{ str: c.l2, font: `400 ${fs2}px ${c.stack}`, color: lt.text2, base: top + h1 + h2 * 0.5 + fs2 * 0.35, align: 'left', inset: pad }] });
  }
  return place(pieces, gw, c, 'left');   // always the full width between the margins
}
function buildLocation(u, c) {
  const pad = 26 * u, s1 = 44 * u, s2 = 28 * u;
  const f1 = `700 ${s1}px ${c.stack}`, f2 = `400 ${s2}px ${c.stack}`;
  const h = c.l2 ? s1 * 1.15 + s2 * 1.3 + pad * 1.1 : s1 + pad * 1.6;
  const boxW = Math.max(textW(c.l1, f1), c.l2 ? textW(c.l2, f2) : 0) + 2 * pad;
  const top = c.bottom - h;
  const b1 = c.l2 ? top + pad * 0.55 + s1 * 0.95 : top + h * 0.5 + s1 * 0.36, b2 = b1 + s2 * 1.35;
  const texts = [{ str: c.l1, font: f1, color: lt.text2, base: b1, align: 'left', inset: pad }];
  if (c.l2) texts.push({ str: c.l2, font: f2, color: lt.text2, alpha: 0.78, base: b2, align: 'left', inset: pad });
  const pieces = [
    { x: 0, y: top, w: h, h, fill: lt.accent, pin: lt.tagText, texts: [] },
    { x: h, y: top, w: boxW, h, fill: lt.band, texts },
  ];
  return place(pieces, h + boxW, c);
}
function buildMinimal(u, c) {
  const s1 = 60 * u, s2 = 34 * u, rule = 7 * u;
  const f1 = `700 ${s1}px ${c.stack}`, f2 = `400 ${s2}px ${c.stack}`;
  const w1 = c.l1 ? textW(c.l1, f1) : 0, w2 = c.l2 ? textW(c.l2, f2) : 0;
  const b2 = c.bottom - s2 * 0.3, b1 = c.l2 ? b2 - s2 * 1.45 : c.bottom - s1 * 0.25;
  const ry = b1 - s1 * 0.78 - 22 * u - rule;
  const align = lt.pos === 'center' ? 'center' : lt.pos;
  const at = w => (lt.pos === 'right' ? c.W - c.mx - w : lt.pos === 'center' ? (c.W - w) / 2 : c.mx);
  const pieces = [{ x: at(90 * u), y: ry, w: 90 * u, h: rule, fill: lt.accent, rule: true, texts: [] }];
  if (c.l1) pieces.push({ x: at(w1), y: b1 - s1 * 0.95, w: w1, h: s1 * 1.25, texts: [{ str: c.l1, font: f1, color: lt.text2, base: b1, align, inset: 0 }] });
  if (c.l2) pieces.push({ x: at(w2), y: b2 - s2 * 0.95, w: w2, h: s2 * 1.25, texts: [{ str: c.l2, font: f2, color: lt.text2, alpha: 0.88, base: b2, align, inset: 0 }] });
  return { pieces, groupW: Math.max(90 * u, w1, w2), side: lt.pos, textOnly: true };
}

/* ---------- timeline ---------- */
function ltTimeline(L) {
  const n = Math.max(1, L.pieces.length);
  const inStag = n > 1 ? lt.inDur * 0.35 / (n - 1) : 0, outStag = n > 1 ? lt.outDur * 0.3 / (n - 1) : 0;
  const start = lt.delay, inEnd = start + lt.inDur, outStart = inEnd + lt.hold;
  const total = lt.outOn ? outStart + lt.outDur + 0.2 : inEnd + lt.hold;
  return {
    n, start, inEnd, outStart, total,
    inDur1: lt.inDur - inStag * (n - 1), outDur1: lt.outDur - outStag * (n - 1),
    // Raw progress (0–1) of piece k coming in, and of it going out (last piece in leaves first)
    inAt(k, t) { return this.inDur1 > 0 ? clamp((t - start - k * inStag) / this.inDur1, 0, 1) : (t >= start ? 1 : 0); },
    outAt(k, t) { if (!lt.outOn) return 0; return this.outDur1 > 0 ? clamp((t - outStart - (n - 1 - k) * outStag) / this.outDur1, 0, 1) : (t >= outStart ? 1 : 0); },
  };
}

/* ---------- drawing (shared by preview and export) ---------- */
function drawPin(ctx, cx, cy, S, color) {
  const R = 0.24 * S, hy = cy - 0.08 * S, ty = cy + 0.34 * S, phi = Math.acos(R / (ty - hy));
  ctx.beginPath();
  ctx.moveTo(cx, ty);
  ctx.arc(cx, hy, R, Math.PI / 2 + phi, Math.PI / 2 - phi + Math.PI * 2);
  ctx.closePath();
  ctx.moveTo(cx + R * 0.42, hy);
  ctx.arc(cx, hy, R * 0.42, 0, Math.PI * 2);
  ctx.fillStyle = color; ctx.fill('evenodd');
}
function drawTexts(ctx, p, L, dx) {
  for (const tx of p.texts) {
    if (!tx.str) continue;
    ctx.font = tx.font; ctx.fillStyle = tx.color; ctx.globalAlpha *= tx.alpha ?? 1;
    ctx.textBaseline = 'alphabetic'; ctx.textAlign = tx.align;
    const x = tx.align === 'left' ? p.x + tx.inset : tx.align === 'right' ? p.x + p.w - tx.inset : p.x + p.w / 2;
    if (L.textOnly && lt.shadow) { ctx.shadowColor = 'rgba(0,0,0,0.6)'; ctx.shadowBlur = 18 * L.u; ctx.shadowOffsetY = 3 * L.u; }
    ctx.fillText(tx.str, x + dx, tx.base);
    ctx.globalAlpha /= tx.alpha ?? 1;
  }
}
function drawPiece(ctx, p, k, L, TL, t) {
  const pin = TL.inAt(k, t), pout = TL.outAt(k, t);
  if (pin <= 0 || pout >= 1) return;
  const { u, W, H } = L, side = L.side, dir = side === 'right' ? 1 : -1;
  const eIn = lt.anim === 'pop' ? easeOutBack(pin) : EASE.glide(pin), eOut = EASE.smooth(pout);
  const e = Math.min(eIn, 1 - eOut), raw = Math.min(pin, 1 - pout);
  ctx.save();
  let dx = 0, tdx = 0;
  if (lt.anim === 'wipe') {
    const r = clamp(e, 0, 1), m = 60 * u, w = p.w * r;
    const x = side === 'right' ? p.x + p.w - w : side === 'center' ? p.x + (p.w - w) / 2 : p.x;
    ctx.beginPath(); ctx.rect(x, p.y - m, w, p.h + 2 * m); ctx.clip();
    tdx = (1 - r) * 30 * u * dir * (side === 'center' ? 0 : 1);
  } else if (lt.anim === 'slide') {
    if (side === 'center') { ctx.translate(0, (1 - e) * 90 * u); ctx.globalAlpha = clamp(raw * 2.5, 0, 1); }
    else { dx = side === 'right' ? (1 - e) * (W - p.x + 40 * u) : -(1 - e) * (p.x + p.w + 40 * u); ctx.translate(dx, 0); }
  } else if (lt.anim === 'fade') {
    ctx.globalAlpha = clamp(e, 0, 1); ctx.translate(0, (1 - e) * 22 * u);
  } else { // pop: grows from its side with a little overshoot
    const s = lerp(0.6, 1, e), ox = side === 'right' ? p.x + p.w : side === 'center' ? p.x + p.w / 2 : p.x, oy = p.y + p.h / 2;
    ctx.translate(ox, oy); ctx.scale(s, s); ctx.translate(-ox, -oy);
    ctx.globalAlpha = clamp(raw * 3, 0, 1);
  }
  if (p.fill) {
    if (lt.shadow && !p.rule) { ctx.shadowColor = 'rgba(0,0,0,0.32)'; ctx.shadowBlur = 24 * u; ctx.shadowOffsetY = 6 * u; }
    ctx.fillStyle = p.fill;
    roundRect(ctx, p.x, p.y, p.w, p.h, p.rule ? p.h / 2 : lt.radius * u); ctx.fill();
    ctx.shadowColor = 'transparent'; ctx.shadowBlur = 0; ctx.shadowOffsetY = 0;
  }
  if (p.pin) drawPin(ctx, p.x + p.w / 2, p.y + p.h / 2, Math.min(p.w, p.h) * 0.82, p.pin);
  if (p.texts.length) {
    if (p.fill) { ctx.beginPath(); ctx.rect(p.x, p.y, p.w, p.h); ctx.clip(); }
    drawTexts(ctx, p, L, tdx);
  }
  ctx.restore();
}
/** opts.bg: a color to fill with, or null for transparent. opts.photo: an image drawn under it (preview only). */
function ltDraw(ctx, W, H, t, L, TL, opts = {}) {
  ctx.save();
  ctx.clearRect(0, 0, W, H);
  if (opts.bg) { ctx.fillStyle = opts.bg; ctx.fillRect(0, 0, W, H); }
  if (opts.photo) {
    const img = opts.photo, s = Math.max(W / img.naturalWidth, H / img.naturalHeight);
    ctx.drawImage(img, (W - img.naturalWidth * s) / 2, (H - img.naturalHeight * s) / 2, img.naturalWidth * s, img.naturalHeight * s);
  }
  L.pieces.forEach((p, k) => drawPiece(ctx, p, k, L, TL, t));
  if (L.empty) {
    ctx.fillStyle = 'rgba(255,255,255,0.7)'; ctx.textAlign = 'center'; ctx.font = `600 ${L.minSide * 0.032}px ${FONTS[lt.font]}`;
    ctx.fillText('Type a name or headline', W / 2, H / 2);
  }
  ctx.restore();
}
/** The background a render uses: none for PNG frames with a transparent background, otherwise a color. */
function renderBg(format) {
  if (lt.bgMode === 'color') return lt.bgColor;
  if (lt.bgMode === 'green') return GREEN;
  return format === 'png-seq' ? null : GREEN;
}

/* ---------- preview ---------- */
function ltPaint(t) {
  if (!ltL) return;
  const { W, H } = ltL;
  if (ltCanvas.width !== W || ltCanvas.height !== H) { ltCanvas.width = W; ltCanvas.height = H; }
  const ctx = ltCanvas.getContext('2d');
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  const bg = lt.bgMode === 'transparent' ? null : renderBg(lt.out.format);
  ltDraw(ctx, W, H, t, ltL, ltTL, { bg, photo: bgImg });
  ltViewer.classList.toggle('checker', !bg && !bgImg);
  const g = ltGuides.getContext('2d');
  const dpr = window.devicePixelRatio || 1;
  const gw = Math.round(ltViewer.clientWidth * dpr), gh = Math.round(ltViewer.clientHeight * dpr);
  if (ltGuides.width !== gw || ltGuides.height !== gh) { ltGuides.width = gw; ltGuides.height = gh; }
  g.clearRect(0, 0, gw, gh);
  if (ltPlayer.guides) drawGuides(g, gw, gh, lt.out.aspect, gw / W * 2);
}
function ltRenderTrack() {
  const tr = $('#lt-track'), T = ltTL.total;
  const blk = (from, len, c, label, dim) => `<div class="blk${dim ? ' dim' : ''}" style="left:${from / T * 100}%;width:${Math.max(0.3, len / T * 100)}%;--c:${c}">${label ? `<span>${label}</span>` : ''}</div>`;
  let html = blk(ltTL.start, ltTL.inEnd - ltTL.start, lt.accent, 'In');
  html += blk(ltTL.inEnd, ltTL.outStart - ltTL.inEnd, '#8a857c', 'Hold', true);
  if (lt.outOn) html += blk(ltTL.outStart, lt.outDur, lt.accent, 'Out');
  tr.innerHTML = html + '<div class="head"></div>';
}
const STYLE_NAMES = { bar: 'name bar', breaking: 'breaking strap', location: 'location tag', minimal: 'minimal' };
function ltRefresh() {
  ltL = ltLayout();
  ltTL = ltTimeline(ltL);
  const { OW, OH } = dims(lt.out);
  $('#lt-fmt').textContent = `${OW} × ${OH} · ${STYLE_NAMES[lt.style]} · ${fmtSec(ltTL.total)} · ${lt.out.fps} fps`;
  toolLt.dataset.style = lt.style;
  const loc = lt.style === 'location', min = lt.style === 'minimal';
  $('.lt-band-lbl').textContent = loc ? 'Box' : 'Lower bar';
  $('.lt-text2-lbl').textContent = loc || min ? 'Text' : 'Lower line text';
  $('.lt-tagtext-lbl').textContent = loc ? 'Pin' : 'Tag text';
  $('#lt-bgcolor-field').hidden = lt.bgMode !== 'color';
  $('#lt-outnote').textContent = lt.bgMode === 'transparent' && lt.out.format === 'mp4'
    ? 'MP4 can’t store transparency, so the video renders on green screen for keying. Choose PNG frames to keep the background transparent.'
    : lt.bgMode === 'transparent' ? 'PNG frames keep the background transparent: drop the folder into your editor as an image sequence.' : '';
  ltRenderTrack();
  if (ltPlayer.t > ltTL.total) ltPlayer.seek(ltTL.total); else { ltPaint(ltPlayer.t); ltPlayer.sync(); }
  saveSoon('lt', lt);
}
function ltFit() { fitViewer(ltMonitor, ltViewer, lt.out.aspect); ltPaint(ltPlayer.t); }
async function ltEnsureFonts() {
  try { await Promise.all([400, 700].map(w => document.fonts.load(`${w} 40px ${FONTS[lt.font]}`, 'AaBbGg09'))); } catch (e) { /* fall back silently */ }
}
/** The moment the graphic is fully on screen: where the preview rests and what Save this frame captures by default. */
const restTime = () => Math.min(ltTL.total, ltTL.inEnd + Math.min(0.5, lt.hold));

/* ---------- controls ---------- */
const ltChanged = key => {
  if (key === 'out.aspect') ltFit();
  if (key === 'font') ltEnsureFonts().then(ltRefresh);
  ltRefresh();
  // Changing the look while parked where the graphic is off screen shows nothing; jump to where it's visible
  if (!ltPlayer.playing && !/^(delay|inDur|outDur|hold|outOn)$/.test(key) && (ltPlayer.t < ltTL.inEnd || ltPlayer.t > ltTL.outStart)) ltPlayer.seek(restTime());
};
bindSeg('lt-style', lt, 'style', ltChanged);
bindText('lt-line1', lt, 'line1', ltChanged);
bindText('lt-line2', lt, 'line2', ltChanged);
bindText('lt-tag', lt, 'tag', ltChanged);
bindCheck('lt-caps1', lt, 'caps1', ltChanged);
bindCheck('lt-caps2', lt, 'caps2', ltChanged);
bindSelect('lt-font', lt, 'font', ltChanged);
bindRange('lt-size', lt, 'size', v => `${Math.round(v * 100)}%`, ltChanged);
bindSeg('lt-pos', lt, 'pos', ltChanged);
bindRange('lt-lift', lt, 'lift', v => `${Math.round(v * 100)}%`, ltChanged);
bindRange('lt-radius', lt, 'radius', v => `${v}px`, ltChanged);
bindCheck('lt-shadow', lt, 'shadow', ltChanged);
for (const k of ['bar', 'text1', 'band', 'text2', 'accent', 'tagText']) bindColor('lt-' + k.toLowerCase(), lt, k, ltChanged);
bindSeg('lt-bgmode', lt, 'bgMode', ltChanged);
bindColor('lt-bgcolor', lt, 'bgColor', ltChanged);
bindSeg('lt-anim', lt, 'anim', ltChanged);
bindRange('lt-delay', lt, 'delay', fmtSec, ltChanged);
bindRange('lt-indur', lt, 'inDur', fmtSec, ltChanged);
bindRange('lt-hold', lt, 'hold', fmtSec, ltChanged);
bindCheck('lt-outon', lt, 'outOn', ltChanged);
bindRange('lt-outdur', lt, 'outDur', fmtSec, ltChanged);

// a photo of your own wins over the app's video (inside ClipCut), which comes back when it is removed
let ownPhoto = false;
function showBackdrop() {
  bgImg = hosted.backdrop;
  $('#lt-photo-name').textContent = bgImg ? 'Your video at the playhead' : '';
  $('#lt-photo-clear').hidden = true;
}
$('#lt-photo-pick').addEventListener('click', () => $('#lt-photo-file').click());
$('#lt-photo-file').addEventListener('change', async e => {
  const f = e.target.files[0]; e.target.value = '';
  if (!f) return;
  try { bgImg = await loadImageFile(f); ownPhoto = true; $('#lt-photo-name').textContent = f.name; $('#lt-photo-clear').hidden = false; ltPaint(ltPlayer.t); }
  catch (err) { toast('That file couldn’t be opened as an image. Try a JPEG or PNG.'); }
});
$('#lt-photo-clear').addEventListener('click', () => { ownPhoto = false; showBackdrop(); ltPaint(ltPlayer.t); });
hosted.onBackdrop(() => { if (!ownPhoto) { showBackdrop(); ltPaint(ltPlayer.t); } });

const ltPlayer = makePlayer('lt', { total: () => (ltTL ? ltTL.total : 1), fps: () => lt.out.fps, render: ltPaint, track: $('#lt-track') });

/* ---------- export ---------- */
async function ltRenderOne() {
  ltPlayer.pause();
  await ltEnsureFonts(); ltRefresh();
  if (ltL.empty) { toast('Type a name or headline first.'); return 'error'; }
  const L = ltL, TL = ltTL, { q, OW, OH } = dims(lt.out), bg = renderBg(lt.out.format);
  return runRender({
    title: `Rendering ${STYLE_NAMES[lt.style]}`, OW, OH, fps: lt.out.fps, total: TL.total, format: lt.out.format,
    name: `lowerthird_${slug(lt.line1 || lt.line2)}_${OW}x${OH}`,
    frame: (ctx, t) => { ctx.setTransform(q, 0, 0, q, 0, 0); ltDraw(ctx, L.W, L.H, t, L, TL, { bg }); },
  });
}
async function ltStill() {
  await ltEnsureFonts(); ltRefresh();
  const { q, OW, OH } = dims(lt.out);
  const c = document.createElement('canvas'); c.width = OW; c.height = OH;
  const ctx = c.getContext('2d'); ctx.setTransform(q, 0, 0, q, 0, 0);
  // A still keeps transparency unless a background was chosen
  const t = ltPlayer.t < ltTL.inEnd || ltPlayer.t >= ltTL.outStart ? restTime() : ltPlayer.t;
  ltDraw(ctx, ltL.W, ltL.H, t, ltL, ltTL, { bg: lt.bgMode === 'transparent' ? null : renderBg('png-seq') });
  await saveStill(c, `lowerthird_${slug(lt.line1 || lt.line2)}_${OW}x${OH}.png`);
}
buildOutputPanel('lt', lt.out, ltChanged, {
  render: ltRenderOne,
  renderBoth: () => renderBoth(lt.out, () => { ltFit(); ltRefresh(); }, ltRenderOne),
  still: ltStill,
});
buildPresetPanel('lt', lt, LT_PRESET_KEYS, LT_BUILTINS, () => { resync(lt); resync(lt.out); ltEnsureFonts().then(ltRefresh); ltFit(); ltRefresh(); ltPlayer.seek(restTime()); });
new ResizeObserver(() => { if (!toolLt.hidden) ltFit(); }).observe(ltMonitor);

CS.tools.lt = {
  id: 'lt',
  player: ltPlayer,
  boot() { ltRefresh(); ltPlayer.seek(restTime()); ltEnsureFonts().then(ltRefresh); },
  show() { requestAnimationFrame(ltFit); },
  hide() { ltPlayer.pause(); },
  debug: { lt, ltPlayer, get ltL() { return ltL; }, get ltTL() { return ltTL; }, ltRenderBg: renderBg },
};
})();
