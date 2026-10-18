/* Clip Studio · Underline tool
   Classic script (not an ES module) so index.html still works when opened straight from disk.
   Shared helpers live on window.CS; each tool registers itself in CS.tools. */
(() => {
'use strict';
const {
  $, $$, clamp, lerp, smoothstep, sleep, EASE, ease, easeOutBounce, easeOutBack, mulberry32, pad2, fmtTC, fmtSec, isHex, slug, clone, store, toastTimer, toast, roundRect, loadImageFile, FRAMES, dims, SWATCHES, registry, reg, resync, saveTimers, saveSoon, bindRange, bindColor, bindCheck, bindSeg, bindText, bindSelect, buildOutputPanel, userPresets, presetUI, buildPresetPanel, ICON_PLAY, ICON_PAUSE, ICON_START, makePlayer, fitViewer, drawGuides, pickEncoder, mp4Sink, pngSink, downloadBlob, fmtBytes, render, runRender, renderBoth, saveStill,
} = window.CS;

/* ============================================================
   UNDERLINE TOOL
   ============================================================ */
const SRC_STACK = '"Libre Franklin", "Helvetica Neue", Arial, sans-serif';
const FONTS = [
  { name: 'Playfair Display', group: 'Serif', stack: '"Playfair Display", Georgia, serif' },
  { name: 'Merriweather', group: 'Serif', stack: 'Merriweather, Georgia, serif' },
  { name: 'Libre Caslon Text', group: 'Serif', stack: '"Libre Caslon Text", Georgia, serif' },
  { name: 'Source Serif 4', group: 'Serif', stack: '"Source Serif 4", Georgia, serif' },
  { name: 'DM Serif Display', group: 'Serif', stack: '"DM Serif Display", Georgia, serif' },
  { name: 'Libre Franklin', group: 'Sans serif', stack: '"Libre Franklin", Arial, sans-serif' },
  { name: 'Inter', group: 'Sans serif', stack: 'Inter, Arial, sans-serif' },
  { name: 'Roboto', group: 'Sans serif', stack: 'Roboto, Arial, sans-serif' },
  { name: 'Archivo', group: 'Sans serif', stack: 'Archivo, Arial, sans-serif' },
  { name: 'Montserrat', group: 'Sans serif', stack: 'Montserrat, Arial, sans-serif' },
  { name: 'Roboto Condensed', group: 'Condensed', stack: '"Roboto Condensed", "Arial Narrow", sans-serif' },
  { name: 'Oswald', group: 'Condensed', stack: 'Oswald, "Arial Narrow", sans-serif' },
  { name: 'Bebas Neue', group: 'Condensed', stack: '"Bebas Neue", Impact, sans-serif' },
  { name: 'Georgia', group: 'Built into your computer', stack: 'Georgia, serif' },
  { name: 'Times New Roman', group: 'Built into your computer', stack: '"Times New Roman", Times, serif' },
  { name: 'Arial', group: 'Built into your computer', stack: 'Arial, Helvetica, sans-serif' },
  { name: 'Helvetica', group: 'Built into your computer', stack: 'Helvetica, Arial, sans-serif' },
];

const UL_SAMPLE = 'City council approves $2.4 billion transit plan after six-hour debate';
const sampleSpan = phrase => { const i = UL_SAMPLE.indexOf(phrase); return { start: i, end: i + phrase.length, color: null }; };
const UL_DEFAULT = {
  src: 'paste', mode: 'text',
  text: UL_SAMPLE,
  spans: [sampleSpan('approves $2.4 billion transit plan'), sampleSpan('six-hour debate')],
  source: 'The Daily Ledger · October 14, 2026',
  quote: false, quoteName: '', quoteTitle: '',
  style: 'underline', color: '#FF3B30', thick: 9, offset: 14, hlHeight: 55, hlOpacity: 0.9, hlTextOn: false, hlText: '#FFFFFF',
  bg: '#0F1B2D', transparent: false, bgFit: 'grid', bgGrid: 'even', bgCols: 4, bgGap: 8, bgDim: 0.55, bgBlur: 0, bgDrift: true, fg: '#141414', srcColor: '#6E6A64', card: true, cardColor: '#FFFFFF', shotFit: 'contain',
  font: 'Playfair Display', customFont: '', weight: 700, size: 96, srcSize: 28, lineHeight: 1.12, align: 'left', valign: 'center', autoFit: true,
  intro: 'fade', wordGap: 0.12, delay: 0.4, draw: 0.9, gap: 0.3, hold: 1.6, ease: 'smooth', order: 'sequence', matchSpeed: false,
  camera: 'push', push: 4, followMax: 2.2, followReturn: true,
  out: { aspect: '16:9', quality: '1080', fps: 30, format: 'mp4' },
};
const UL_PRESET_KEYS = ['style', 'color', 'thick', 'offset', 'hlHeight', 'hlOpacity', 'hlTextOn', 'hlText', 'bg', 'transparent', 'bgFit', 'bgGrid', 'bgCols', 'bgGap', 'bgDim', 'bgBlur', 'bgDrift', 'fg', 'srcColor', 'card', 'cardColor', 'shotFit', 'quote', 'font', 'customFont', 'weight', 'size', 'srcSize', 'lineHeight', 'align', 'valign', 'autoFit', 'intro', 'wordGap', 'delay', 'draw', 'gap', 'hold', 'ease', 'order', 'matchSpeed', 'camera', 'push', 'followMax', 'followReturn', 'out'];
const UL_BUILTINS = {
  'News red underline': { style: 'underline', color: '#FF3B30', thick: 9, offset: 14, bg: '#0F1B2D', fg: '#141414', srcColor: '#6E6A64', card: true, cardColor: '#FFFFFF', quote: false, font: 'Playfair Display', weight: 700, size: 96, lineHeight: 1.12, align: 'left', valign: 'center', intro: 'fade', draw: 0.9, gap: 0.3, hold: 1.6, ease: 'smooth', camera: 'push', push: 4 },
  'Headline over a photo grid': { style: 'highlight', color: '#FFD60A', hlHeight: 58, hlOpacity: 0.95, hlTextOn: false, bg: '#0B0B0C', fg: '#111111', card: true, cardColor: '#FFFFFF', bgFit: 'grid', bgGrid: 'tilted', bgCols: 4, bgGap: 10, bgDim: 0.6, bgBlur: 1, bgDrift: true, intro: 'rise', ease: 'snappy', camera: 'push', push: 4 },
  'Yellow highlighter': { style: 'highlight', color: '#FFD60A', hlHeight: 58, hlOpacity: 0.95, hlTextOn: false, bg: '#E6E1D7', fg: '#111111', srcColor: '#6E6A64', card: true, cardColor: '#FFFFFF', quote: false, font: 'Libre Franklin', weight: 700, size: 92, lineHeight: 1.15, align: 'left', intro: 'none', draw: 0.7, gap: 0.25, ease: 'snappy', camera: 'push', push: 6 },
  'Quote card': { style: 'marker', color: '#F4C443', thick: 10, offset: 16, bg: '#111111', fg: '#F5F1EA', srcColor: '#A39D93', card: false, quote: true, font: 'DM Serif Display', weight: 400, size: 100, lineHeight: 1.15, align: 'left', valign: 'center', intro: 'rise', draw: 1.0, ease: 'smooth', camera: 'still' },
  'Vertical punch (9:16)': { style: 'highlight', color: '#FF3B30', hlHeight: 100, hlOpacity: 1, hlTextOn: true, hlText: '#FFFFFF', bg: '#000000', fg: '#FFFFFF', srcColor: '#A39D93', card: false, quote: false, font: 'Bebas Neue', weight: 400, size: 150, lineHeight: 1.0, align: 'center', valign: 'center', intro: 'rise', draw: 0.5, gap: 0.15, ease: 'snappy', camera: 'follow', followMax: 1.5, followReturn: false, out: { aspect: '9:16' } },
  'Word-by-word quote': { style: 'marker', color: '#F4C443', thick: 10, offset: 16, bg: '#111111', fg: '#F5F1EA', srcColor: '#A39D93', card: false, quote: true, font: 'DM Serif Display', weight: 400, size: 96, lineHeight: 1.15, align: 'left', valign: 'center', intro: 'words', wordGap: 0.14, delay: 0.3, draw: 0.9, ease: 'smooth', camera: 'push', push: 3 },
  'Screenshot callout': { style: 'circle', color: '#FF3B30', thick: 8, bg: '#1A1A1A', card: true, shotFit: 'contain', intro: 'fade', draw: 1.0, gap: 0.4, hold: 1.6, ease: 'smooth', camera: 'follow', followMax: 2.4, followReturn: true },
};

const ul = Object.assign(clone(UL_DEFAULT), store.get('ul', {}));
ul.out = Object.assign({}, UL_DEFAULT.out, ul.out || {});
if (!FRAMES[ul.out.aspect]) ul.out.aspect = '16:9';
ul.spans = (Array.isArray(ul.spans) ? ul.spans : []).filter(s => s && s.end > s.start && s.end <= ul.text.length);
if (!FONTS.some(f => f.name === ul.font)) ul.font = 'Playfair Display';
if (ul.src === 'shot') { ul.src = 'paste'; } // screenshots aren't kept between sessions
ul.mode = ul.src === 'shot' ? 'shot' : 'text';

const shot = { img: null, name: '', marks: [], edit: false, drag: null };
let headshot = null;
let bgImg = null;   // background image; like other media it isn't kept between sessions
const measureCtx = document.createElement('canvas').getContext('2d');
let ulL = null, ulTL = null, lastAdded = null;

function ulFontStack() {
  const custom = (ul.customFont || '').trim().replace(/["\\]/g, '');
  const f = FONTS.find(x => x.name === ul.font) || FONTS[0];
  return custom ? `"${custom}", ${f.stack}` : f.stack;
}
async function ulEnsureFonts() {
  const st = ulFontStack();
  try {
    await Promise.all([
      document.fonts.load(`${ul.weight} 64px ${st}`, ul.text.slice(0, 60) || 'Ag'),
      document.fonts.load(`600 32px ${SRC_STACK}`, 'Ag'),
      document.fonts.load(`700 32px ${SRC_STACK}`, 'Ag'),
    ]);
  } catch (e) { /* fall back silently */ }
}

function wrapText(ctx, text, maxW) {
  const lines = [];
  let idx = 0;
  for (const para of text.split('\n')) {
    const re = /\S+/g; let m; let cur = null;
    const push = c => { const t = text.slice(c.s, c.e); lines.push({ s: c.s, e: c.e, text: t, width: ctx.measureText(t).width }); };
    let any = false;
    while ((m = re.exec(para))) {
      any = true;
      const s = idx + m.index, e = s + m[0].length;
      if (!cur) { cur = { s, e }; continue; }
      if (ctx.measureText(text.slice(cur.s, e)).width <= maxW) cur.e = e;
      else { push(cur); cur = { s, e }; }
    }
    if (cur) push(cur);
    if (!any) lines.push({ s: idx, e: idx, text: '', width: 0 });
    idx += para.length + 1;
  }
  return lines;
}

function ulLayout(ctx) {
  const { W, H } = dims(ul.out);
  const minSide = Math.min(W, H);
  return ul.mode === 'shot' ? shotLayout(W, H, minSide) : textLayout(ctx, W, H, minSide);
}

function textLayout(ctx, W, H, minSide) {
  const pad = Math.round(minSide * 0.08);
  const cardPad = ul.card ? Math.round(minSide * 0.055) : 0;
  const stack = ulFontStack();
  const srcText = (ul.source || '').trim().toUpperCase();
  const quoteOn = !!ul.quote;
  const qName = quoteOn ? (ul.quoteName || '').trim() : '';
  const qTitle = quoteOn ? (ul.quoteTitle || '').trim() : '';
  const hasAttr = quoteOn && !!(qName || qTitle || headshot);
  const maxW = W - 2 * pad - 2 * cardPad;
  // Source line: the size you set (in pixels at 1080p). A long source wraps onto a second line first,
  // and is made smaller only if two lines still don't fit across the frame.
  const srcWanted = Math.round(ul.srcSize * minSide / 1080);
  let srcSize = srcWanted, srcLines = [];
  if (srcText) {
    for (let i = 0; i < 40; i++) {
      ctx.font = `600 ${srcSize}px ${SRC_STACK}`;
      if ('letterSpacing' in ctx) ctx.letterSpacing = `${(srcSize * 0.09).toFixed(1)}px`;
      srcLines = [];
      let cur = '';
      for (const word of srcText.split(/\s+/)) {
        const next = cur ? cur + ' ' + word : word;
        if (cur && ctx.measureText(next).width > maxW) { srcLines.push(cur); cur = word; } else cur = next;
      }
      if (cur) srcLines.push(cur);
      srcLines = srcLines.map(text => ({ text, w: ctx.measureText(text).width }));
      if ((srcLines.length <= 2 && srcLines.every(l => l.w <= maxW)) || srcSize <= 8) break;
      srcSize = Math.max(8, Math.floor(srcSize * 0.95));
    }
    if ('letterSpacing' in ctx) ctx.letterSpacing = '0px';
  }
  const srcLH = srcSize * 1.35;
  const srcBlock = srcText ? Math.round(srcSize * 2.3 + (srcLines.length - 1) * srcLH) : 0;
  let size = ul.size, lines = [], qBlock = 0, qGlyph = null, attrBlock = 0, attrSize = 0;
  for (let i = 0; i < 60; i++) {
    ctx.font = `${ul.weight} ${size}px ${stack}`;
    lines = wrapText(ctx, ul.text, maxW);
    if (quoteOn) {
      const qs = Math.round(size * 1.9);
      ctx.font = `${ul.weight} ${qs}px ${stack}`;
      const m = ctx.measureText('“');
      const asc = m.actualBoundingBoxAscent || qs * 0.72, dsc = m.actualBoundingBoxDescent || -qs * 0.35;
      qGlyph = { size: qs, asc, h: Math.max(qs * 0.2, asc + dsc) };
      qBlock = Math.round(qGlyph.h + size * 0.12);
    }
    attrSize = Math.max(Math.round(minSide * 0.024), Math.round(size * 0.3));
    attrBlock = hasAttr ? Math.round(size * 0.5 + Math.max(attrSize * 2.4, headshot ? attrSize * 2.9 : 0)) : 0;
    const availH = H - 2 * pad - 2 * cardPad - srcBlock - qBlock - attrBlock;
    const tooTall = lines.length * size * ul.lineHeight > availH;
    const tooWide = lines.some(l => l.width > maxW + 1);
    if (!ul.autoFit || (!tooTall && !tooWide) || size <= 14) break;
    size = Math.max(14, Math.floor(size * 0.95));
  }
  ctx.font = `${ul.weight} ${size}px ${stack}`;
  const mm = ctx.measureText('Hgjy');
  const asc = mm.fontBoundingBoxAscent || size * 0.86;
  const desc = mm.fontBoundingBoxDescent || size * 0.24;
  const xh = ctx.measureText('x').actualBoundingBoxAscent || size * 0.5;
  const lh = size * ul.lineHeight;
  const textH = lines.length * lh;
  const srcW = srcLines.length ? Math.max(...srcLines.map(l => l.w)) : 0;
  // attribution width
  let attr = null;
  if (hasAttr) {
    const d = headshot ? Math.round(attrSize * 2.6) : 0;
    const nameTxt = qName ? (headshot ? qName : '— ' + qName) : '';
    ctx.font = `700 ${attrSize}px ${SRC_STACK}`; const nw = nameTxt ? ctx.measureText(nameTxt).width : 0;
    ctx.font = `500 ${Math.round(attrSize * 0.82)}px ${SRC_STACK}`; const tw = qTitle ? ctx.measureText(qTitle).width : 0;
    attr = { d, nameTxt, title: qTitle, size: attrSize, w: d + (d ? attrSize * 0.7 : 0) + Math.max(nw, tw) };
  }
  const contentW = Math.max(srcW, ...lines.map(l => l.width), attr ? attr.w : 0, 1);
  const innerW = ul.card ? Math.min(maxW, contentW) : maxW;
  const boxW = innerW + 2 * cardPad, boxH = srcBlock + qBlock + textH + attrBlock + 2 * cardPad;
  const boxX = (W - boxW) / 2;
  const boxY = ul.valign === 'top' ? pad : ul.valign === 'bottom' ? H - pad - boxH : (H - boxH) / 2;
  const innerX = boxX + cardPad, innerY = boxY + cardPad;
  const baseOff = (lh - (asc + desc)) / 2 + asc;
  const textTop = innerY + srcBlock + qBlock;
  lines.forEach((ln, i) => {
    ln.i = i;
    ln.x = ul.align === 'center' ? innerX + (innerW - ln.width) / 2 : innerX;
    ln.baseline = textTop + i * lh + baseOff;
  });
  const src = srcText ? { size: srcSize, lh: srcLH, y: innerY + srcSize * 1.05, lines: srcLines.map(l => ({ text: l.text, x: ul.align === 'center' ? innerX + (innerW - l.w) / 2 : innerX })) } : null;
  let quote = null;
  if (quoteOn && qGlyph) {
    ctx.font = `${ul.weight} ${qGlyph.size}px ${stack}`;
    const gw = ctx.measureText('“').width;
    quote = { size: qGlyph.size, x: ul.align === 'center' ? innerX + (innerW - gw) / 2 : innerX - qGlyph.size * 0.03, y: innerY + srcBlock + qGlyph.asc };
  }
  if (attr) {
    const top = textTop + textH + size * 0.5;
    const x0 = ul.align === 'center' ? innerX + (innerW - attr.w) / 2 : innerX;
    const blockH = Math.max(attr.d, attrSize * 2.2);
    attr.cx = x0 + attr.d / 2; attr.cy = top + blockH / 2;
    attr.tx = x0 + (attr.d ? attr.d + attrSize * 0.7 : 0);
    const both = attr.nameTxt && attr.title;
    attr.ny = attr.cy + (both ? -attrSize * 0.12 : attrSize * 0.36);
    attr.ty = both ? attr.cy + attrSize * 1.02 : attr.cy + attrSize * 0.32;
  }

  ctx.font = `${ul.weight} ${size}px ${stack}`;
  // Word positions for the word-by-word reveal (k counts words across all lines)
  let nWords = 0;
  for (const ln of lines) {
    ln.words = [...ln.text.matchAll(/\S+/g)].map(m => ({ text: m[0], x: ln.x + ctx.measureText(ln.text.slice(0, m.index)).width, k: nWords++ }));
  }
  const text = ul.text;
  const spans = ul.spans.slice().sort((a, b) => a.start - b.start).map((sp, si) => {
    const segs = [];
    for (const ln of lines) {
      let a = Math.max(sp.start, ln.s), b = Math.min(sp.end, ln.e);
      if (b <= a) continue;
      while (a < b && /\s/.test(text[a])) a++;
      while (b > a && /\s/.test(text[b - 1])) b--;
      if (b <= a) continue;
      const x0 = ln.x + ctx.measureText(text.slice(ln.s, a)).width;
      const x1 = ln.x + ctx.measureText(text.slice(ln.s, b)).width;
      segs.push({ x0, x1, w: x1 - x0, baseline: ln.baseline, top: ln.baseline - asc * 0.92 - desc * 0.15, bottom: ln.baseline + desc * 0.55, mid: ln.baseline - xh * 0.45, li: ln.i, line: ln });
    }
    return { ref: sp, idx: si, segs, len: segs.reduce((s, g) => s + g.w, 0) };
  }).filter(s => s.segs.length);
  const f = spans[0]?.segs[0];
  const focus = f ? { x: (f.x0 + f.x1) / 2, y: f.mid } : { x: W / 2, y: H / 2 };
  return {
    mode: 'text', W, H, minSide, size, srcShrunk: srcSize < srcWanted ? srcSize : 0, stack, asc, desc, xh, lh, lines, nWords, src, quote, attr, spans, focus,
    hlPadX: size * 0.08, over: size * 0.05,
    box: { x: boxX, y: boxY, w: boxW, h: boxH, r: Math.round(minSide * 0.012) },
    content: { x: 0, y: 0, w: W, h: H },
  };
}

function shotLayout(W, H, minSide) {
  const base = { mode: 'shot', W, H, minSide, lines: [], spans: [], focus: { x: W / 2, y: H / 2 }, content: { x: 0, y: 0, w: W, h: H }, hlPadX: 0, over: minSide * 0.004, size: minSide * 0.05 };
  if (!shot.img) return base;
  const pad = Math.round(minSide * 0.06);
  const iw = shot.img.naturalWidth || shot.img.width, ih = shot.img.naturalHeight || shot.img.height;
  const sc = ul.shotFit === 'width' ? (W - 2 * pad) / iw : Math.min((W - 2 * pad) / iw, (H - 2 * pad) / ih);
  const w = iw * sc, h = ih * sc;
  const x = (W - w) / 2, y = h <= H - 2 * pad ? (H - h) / 2 : pad;
  const spans = shot.marks.map((mk, idx) => {
    const segs = mk.boxes.map(b => {
      const top = y + b.y * sc, bottom = y + (b.y + b.h) * sc;
      return { x0: x + b.x * sc, x1: x + (b.x + b.w) * sc, w: b.w * sc, top, bottom, baseline: bottom - 14, mid: (top + bottom) / 2, li: 0, line: null };
    });
    return { ref: mk, idx, segs, len: segs.reduce((s, g) => s + g.w, 0) };
  }).filter(s => s.segs.length);
  const f = spans[0]?.segs[0];
  const m = minSide * 0.02;
  return {
    ...base,
    img: { el: shot.img, x, y, w, h, sc, iw, ih, r: Math.round(minSide * 0.012) },
    spans,
    focus: f ? { x: (f.x0 + f.x1) / 2, y: f.mid } : { x: W / 2, y: H / 2 },
    content: { x: x - m, y: y - m, w: w + 2 * m, h: h + 2 * m },
  };
}

function ulTimeline(L) {
  const nLines = L.mode === 'shot' ? 1 : L.lines.length;
  const intro = ulIntro(L);
  const introDur = intro === 'fade' ? 0.5 : intro === 'rise' ? 0.5 + 0.07 * Math.max(0, nLines - 1)
    : intro === 'words' ? WORDS_LEAD + Math.max(0, L.nWords - 1) * ul.wordGap + WORD_IN : 0;
  const base = introDur + ul.delay;
  const avg = L.spans.length ? L.spans.reduce((s, x) => s + x.len, 0) / L.spans.length : 1;
  let t = base;
  const items = L.spans.map(sp => {
    const dur = Math.max(0.05, ul.matchSpeed ? ul.draw * (sp.len / avg) : ul.draw);
    const t0 = ul.order === 'together' ? base : t;
    if (ul.order !== 'together') t = t0 + dur + ul.gap;
    return { ...sp, t0, t1: t0 + dur };
  });
  const drawEnd = items.length ? Math.max(...items.map(i => i.t1)) : base;
  const returnTime = ul.camera === 'follow' && ul.followReturn && items.length ? 1.15 : 0;
  return { introDur, base, items, drawEnd, total: Math.max(0.5, drawEnd + Math.max(ul.hold, returnTime)) };
}

/* Camera: view center (cx, cy) and scale s, applied as translate(W/2,H/2) scale(s) translate(-cx,-cy) */
function frameFor(L, segs) {
  let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
  for (const g of segs) { x0 = Math.min(x0, g.x0); x1 = Math.max(x1, g.x1); y0 = Math.min(y0, g.top); y1 = Math.max(y1, g.bottom); }
  const bh = Math.max(1, y1 - y0), bw = Math.max(1, x1 - x0) + bh * 1.2;
  const s = clamp(Math.min(L.W * 0.86 / bw, L.H * 0.6 / (bh * 2.2)), 1, ul.followMax);
  const C = L.content, hw = L.W / (2 * s), hh = L.H / (2 * s);
  let cx = (x0 + x1) / 2, cy = (y0 + y1) / 2;
  cx = C.w <= 2 * hw ? C.x + C.w / 2 : clamp(cx, C.x + hw, C.x + C.w - hw);
  cy = C.h <= 2 * hh ? C.y + C.h / 2 : clamp(cy, C.y + hh, C.y + C.h - hh);
  return { s, cx, cy };
}
function followKeys(L, TL) {
  if (TL._keys) return TL._keys;
  const base = { s: 1, cx: L.W / 2, cy: L.H / 2 };
  const MOVE = 0.7;
  const groups = ul.order === 'together'
    ? [{ t0: TL.items[0].t0, segs: TL.items.flatMap(i => i.segs) }]
    : TL.items.map(i => ({ t0: i.t0, segs: i.segs }));
  const keys = [{ t: 0, c: base }];
  let prev = base;
  for (const g of groups) {
    const c = frameFor(L, g.segs);
    const last = keys[keys.length - 1].t;
    const tStart = Math.max(last, g.t0 - MOVE);
    keys.push({ t: tStart, c: prev });
    keys.push({ t: Math.max(tStart + 0.05, g.t0), c });
    prev = c;
  }
  if (ul.followReturn) { const t1 = TL.drawEnd + 0.25; keys.push({ t: t1, c: prev }); keys.push({ t: t1 + 0.9, c: base }); }
  TL._keys = keys;
  return keys;
}
function ulCamera(L, TL, t) {
  const base = { s: 1, cx: L.W / 2, cy: L.H / 2 };
  if (ul.camera === 'push' && ul.push > 0) {
    const k = 1 + (ul.push / 100) * EASE.smooth(clamp(t / TL.total, 0, 1));
    return { s: k, cx: L.focus.x + (L.W / 2 - L.focus.x) / k, cy: L.focus.y + (L.H / 2 - L.focus.y) / k };
  }
  if (ul.camera !== 'follow' || !TL.items.length) return base;
  const keys = followKeys(L, TL);
  let i = 0;
  while (i < keys.length - 1 && keys[i + 1].t <= t) i++;
  const a = keys[i], b = keys[i + 1];
  if (!b) return a.c;
  const e = EASE.smooth(clamp((t - a.t) / Math.max(1e-3, b.t - a.t), 0, 1));
  return { s: Math.exp(lerp(Math.log(a.c.s), Math.log(b.c.s), e)), cx: lerp(a.c.cx, b.c.cx, e), cy: lerp(a.c.cy, b.c.cy, e) };
}

function ellipseLen(rx, ry) { const h = Math.pow(rx - ry, 2) / Math.pow(rx + ry, 2); return Math.PI * (rx + ry) * (1 + 3 * h / (10 + Math.sqrt(4 - 3 * h))); }
function circleGeom(g, th) {
  const hgt = g.bottom - g.top;
  const padX = Math.max(th * 1.5, hgt * 0.35), padY = hgt * 0.3 + th;
  return { cx: (g.x0 + g.x1) / 2, cy: (g.top + g.bottom) / 2, rx: g.w / 2 + padX, ry: hgt / 2 + padY };
}

/* Background image: fills the frame or repeats as a grid, tinted with the background color.
   Each tile is scaled (and blurred) once into a cached bitmap, so drawing a frame is a few dozen drawImage calls. */
const bgTileCache = { key: '', c: null };
function bgTile(w, h, k, blur) {
  const key = [bgImg.src, Math.round(w * k), Math.round(h * k), blur].join('|');
  if (bgTileCache.key === key) return bgTileCache.c;
  const c = document.createElement('canvas');
  c.width = Math.max(1, Math.round(w * k)); c.height = Math.max(1, Math.round(h * k));
  const g = c.getContext('2d');
  const iw = bgImg.naturalWidth, ih = bgImg.naturalHeight;
  const over = blur ? 1 + blur * 4 / Math.min(w, h) : 1;   // overscan so blurred edges don't fade to transparent
  const s = Math.max(c.width / iw, c.height / ih) * over;
  if (blur && 'filter' in g) g.filter = `blur(${blur * k}px)`;
  g.imageSmoothingQuality = 'high';
  g.drawImage(bgImg, (c.width - iw * s) / 2, (c.height - ih * s) / 2, iw * s, ih * s);
  bgTileCache.key = key; bgTileCache.c = c;
  return c;
}
function drawBgImage(ctx, W, H, t, total) {
  const k = Math.abs(ctx.getTransform().a) || 1;
  const iw = bgImg.naturalWidth, ih = bgImg.naturalHeight;
  ctx.save();
  if (ul.bgFit === 'fill') {
    const z = ul.bgDrift ? 1 + 0.08 * clamp(t / Math.max(1, total), 0, 1) : 1;  // slow push in
    const base = Math.max(W / iw, H / ih), s = base * z;
    const tile = bgTile(iw * base * 1.08, ih * base * 1.08, k, ul.bgBlur);  // made once at the largest size, so the push-in doesn't rebuild it
    ctx.drawImage(tile, (W - iw * s) / 2, (H - ih * s) / 2, iw * s, ih * s);
  } else {
    const cols = clamp(Math.round(ul.bgCols), 1, 8), gap = ul.bgGap;
    const tw = (W - gap * (cols - 1)) / cols, th = tw * ih / iw;
    const px = tw + gap, py = th + gap;
    const tile = bgTile(tw, th, k, ul.bgBlur);
    const tilted = ul.bgGrid === 'tilted';
    if (tilted) { ctx.translate(W / 2, H / 2); ctx.rotate(-12 * Math.PI / 180); ctx.translate(-W / 2, -H / 2); }
    // Drift: the whole grid slides up and to the left by one tile every 8 seconds
    const ox = ul.bgDrift ? -(t / 8) * px : 0, oy = ul.bgDrift ? -(t / 8) * py : 0;
    const reach = tilted ? Math.hypot(W, H) / 2 + Math.max(px, py) : Math.max(W, H);   // cover the frame even when rotated
    const cx = W / 2, cy = H / 2;
    const r0 = Math.floor((cy - reach - oy) / py) - 1, r1 = Math.ceil((cy + reach - oy) / py) + 1;
    for (let r = r0; r <= r1; r++) {
      const shift = ul.bgGrid === 'brick' && r % 2 ? px / 2 : 0;
      const c0 = Math.floor((cx - reach - ox - shift) / px) - 1, c1 = Math.ceil((cx + reach - ox - shift) / px) + 1;
      for (let c = c0; c <= c1; c++) {
        if (ul.bgGrid === 'checker' && (r + c) % 2) continue;
        ctx.drawImage(tile, c * px + ox + shift, r * py + oy, tw, th);
      }
    }
  }
  ctx.restore();
  if (ul.bgDim > 0) { ctx.save(); ctx.globalAlpha = ul.bgDim; ctx.fillStyle = ul.bg; ctx.fillRect(0, 0, W, H); ctx.restore(); }
}

/* Word by word: the card fades in over WORDS_LEAD, then word k rises in over WORD_IN, starting WORDS_LEAD + k × wordGap.
   Screenshots have no words, so they fade in instead. */
const WORDS_LEAD = 0.3, WORD_IN = 0.3;
const ulIntro = L => (ul.intro === 'words' && L.mode !== 'text' ? 'fade' : ul.intro);
function wordState(L, k, t) {
  const e = EASE.snappy(clamp((t - WORDS_LEAD - k * ul.wordGap) / WORD_IN, 0, 1));
  return { a: e, dy: (1 - e) * L.lh * 0.22 };
}

function ulDraw(ctx, L, TL, t, { fillBg = true, edit = false } = {}) {
  const { W, H } = L;
  ctx.save();
  ctx.clearRect(0, 0, W, H);
  if (fillBg) { ctx.fillStyle = ul.bg; ctx.fillRect(0, 0, W, H); }
  if (fillBg && bgImg) drawBgImage(ctx, W, H, edit ? 0 : t, TL.total);
  if (!edit) { const c = ulCamera(L, TL, t); ctx.translate(W / 2, H / 2); ctx.scale(c.s, c.s); ctx.translate(-c.cx, -c.cy); }
  const intro = ulIntro(L);
  const lineState = i => {
    if (edit) return { a: 1, dy: 0 };
    if (intro === 'words') {
      // i < 0: card and source line; i past the last line: the attribution, which follows the last word
      const t0 = i < 0 ? 0 : i >= L.lines.length ? WORDS_LEAD + Math.max(0, L.nWords - 1) * ul.wordGap : null;
      if (t0 == null) return { a: 1, dy: 0 };
      const e = EASE.snappy(clamp((t - t0) / WORDS_LEAD, 0, 1)); return { a: e, dy: 0 };
    }
    if (intro === 'fade') { const e = EASE.snappy(clamp(t / 0.5, 0, 1)); return { a: e, dy: 0 }; }
    if (intro === 'rise') { const e = EASE.snappy(clamp((t - Math.max(0, i) * 0.07) / 0.5, 0, 1)); return { a: e, dy: (1 - e) * (L.lh || L.minSide * 0.1) * 0.35 }; }
    return { a: 1, dy: 0 };
  };

  if (L.mode === 'shot') {
    if (L.img) {
      const st = lineState(0);
      ctx.save();
      ctx.globalAlpha = st.a;
      const { x, w, h, r } = L.img, y = L.img.y + st.dy;
      if (ul.card) {
        ctx.save(); ctx.shadowColor = 'rgba(0,0,0,0.35)'; ctx.shadowBlur = L.minSide * 0.04; ctx.shadowOffsetY = L.minSide * 0.012;
        ctx.fillStyle = '#ffffff'; roundRect(ctx, x, y, w, h, r); ctx.fill(); ctx.restore();
        roundRect(ctx, x, y, w, h, r); ctx.clip();
      }
      ctx.drawImage(L.img.el, x, y, w, h);
      ctx.restore();
    } else {
      ctx.fillStyle = 'rgba(255,255,255,0.55)';
      ctx.font = `600 ${Math.round(L.minSide * 0.035)}px Barlow, Arial, sans-serif`;
      ctx.textAlign = 'center';
      ctx.fillText('Choose a screenshot to start', W / 2, H / 2);
      ctx.textAlign = 'left';
    }
  } else {
    const cs = lineState(-1);
    if (ul.card) {
      ctx.save();
      ctx.globalAlpha = cs.a;
      ctx.shadowColor = 'rgba(0,0,0,0.30)'; ctx.shadowBlur = L.minSide * 0.045; ctx.shadowOffsetY = L.minSide * 0.012;
      ctx.fillStyle = ul.cardColor;
      roundRect(ctx, L.box.x, L.box.y, L.box.w, L.box.h, L.box.r); ctx.fill();
      ctx.restore();
    }
    if (L.src) {
      ctx.save();
      ctx.globalAlpha = cs.a;
      ctx.font = `600 ${L.src.size}px ${SRC_STACK}`;
      if ('letterSpacing' in ctx) ctx.letterSpacing = `${(L.src.size * 0.09).toFixed(1)}px`;
      ctx.fillStyle = ul.srcColor; ctx.textBaseline = 'alphabetic';
      L.src.lines.forEach((ln, k) => ctx.fillText(ln.text, ln.x, L.src.y + k * L.src.lh + cs.dy));
      ctx.restore();
    }
    if (L.quote) {
      ctx.save();
      ctx.globalAlpha = cs.a;
      ctx.font = `${ul.weight} ${L.quote.size}px ${L.stack}`;
      ctx.fillStyle = ul.color; ctx.textBaseline = 'alphabetic';
      ctx.fillText('“', L.quote.x, L.quote.y + cs.dy);
      ctx.restore();
    }
  }

  const prog = it => (edit ? 1 : t <= it.t0 ? 0 : t >= it.t1 ? 1 : ease(ul.ease, (t - it.t0) / (it.t1 - it.t0)));
  const colorOf = it => it.ref.color || ul.color;
  const hlRects = [];
  const th = ul.thick;

  // Highlight: behind the text for typed text; multiplied over the image for screenshots
  if (ul.style === 'highlight') {
    for (const it of TL.items) {
      const p = prog(it); if (p <= 0) continue;
      const lens = it.segs.map(g => g.w + L.hlPadX * 2);
      let remain = p * lens.reduce((a, b) => a + b, 0);
      ctx.save();
      ctx.globalAlpha = ul.hlOpacity; ctx.fillStyle = colorOf(it);
      if (L.mode === 'shot') ctx.globalCompositeOperation = 'multiply';
      it.segs.forEach((g, gi) => {
        if (remain <= 0) return;
        const take = Math.min(lens[gi], remain); remain -= take;
        const st = lineState(g.li);
        const full = g.bottom - g.top;
        const h = L.mode === 'shot' ? full : full * ul.hlHeight / 100;
        const bottom = g.bottom + st.dy, top = bottom - h;
        const r = { x: g.x0 - L.hlPadX, y: top, w: take, h, line: g.line };
        roundRect(ctx, r.x, r.y, r.w, r.h, Math.min(h * 0.16, L.size * 0.05)); ctx.fill();
        hlRects.push(r);
      });
      ctx.restore();
    }
  }

  // Text, quote attribution
  if (L.mode === 'text') {
    ctx.font = `${ul.weight} ${L.size}px ${L.stack}`; ctx.textBaseline = 'alphabetic'; ctx.fillStyle = ul.fg;
    for (const ln of L.lines) {
      if (intro === 'words' && !edit) {
        for (const w of ln.words) {
          const st = wordState(L, w.k, t);
          if (st.a <= 0) continue;
          ctx.globalAlpha = st.a;
          ctx.fillText(w.text, w.x, ln.baseline + st.dy);
        }
        continue;
      }
      const st = lineState(ln.i);
      ctx.globalAlpha = st.a;
      ctx.fillText(ln.text, ln.x, ln.baseline + st.dy);
    }
    ctx.globalAlpha = 1;
    if (ul.style === 'highlight' && ul.hlTextOn) {
      ctx.fillStyle = ul.hlText;
      for (const r of hlRects) {
        ctx.save(); ctx.beginPath(); ctx.rect(r.x, r.y, r.w, r.h); ctx.clip();
        ctx.fillText(r.line.text, r.line.x, r.line.baseline + lineState(r.line.i).dy);
        ctx.restore();
      }
    }
    if (L.attr) {
      const A = L.attr, st = lineState(L.lines.length);
      ctx.save();
      ctx.globalAlpha = st.a;
      if (A.d && headshot) {
        ctx.save(); ctx.beginPath(); ctx.arc(A.cx, A.cy + st.dy, A.d / 2, 0, Math.PI * 2); ctx.clip();
        const iw = headshot.naturalWidth, ih = headshot.naturalHeight, k = Math.max(A.d / iw, A.d / ih);
        ctx.drawImage(headshot, A.cx - iw * k / 2, A.cy + st.dy - ih * k / 2, iw * k, ih * k);
        ctx.restore();
      }
      if (A.nameTxt) { ctx.font = `700 ${A.size}px ${SRC_STACK}`; ctx.fillStyle = ul.fg; ctx.fillText(A.nameTxt, A.tx, A.ny + st.dy); }
      if (A.title) { ctx.font = `500 ${Math.round(A.size * 0.82)}px ${SRC_STACK}`; ctx.fillStyle = ul.srcColor; ctx.fillText(A.title, A.tx, A.ty + st.dy); }
      ctx.restore();
    }
  }

  // Strokes drawn over the content
  if (ul.style !== 'highlight') {
    ctx.lineCap = 'round'; ctx.lineJoin = 'round';
    for (const it of TL.items) {
      const p = prog(it); if (p <= 0) continue;
      ctx.strokeStyle = colorOf(it);
      const lens = it.segs.map(g => {
        if (ul.style === 'circle') { const c = circleGeom(g, th); return ellipseLen(c.rx, c.ry) * 1.1; }
        if (ul.style === 'marker') return g.w + L.over * 2;
        return Math.max(0.01, g.w - th);
      });
      let remain = p * lens.reduce((a, b) => a + b, 0);
      it.segs.forEach((g, gi) => {
        if (remain <= 0) return;
        const take = Math.min(lens[gi], remain); remain -= take;
        const st = lineState(g.li);
        const rnd = mulberry32((it.idx + 1) * 977 + gi * 131);
        ctx.lineWidth = th;
        if (ul.style === 'circle') {
          const c = circleGeom(g, th);
          const frac = take / lens[gi];
          const a0 = -Math.PI * 0.62 + (rnd() - 0.5) * 0.3, sweep = Math.PI * 2 * 1.1 * frac, ph = rnd() * 6.283;
          const n = Math.max(2, Math.ceil(90 * frac));
          ctx.beginPath();
          for (let k = 0; k <= n; k++) {
            const u = k / n, a = a0 + sweep * u;
            const grow = 1 + 0.06 * u * frac + 0.025 * Math.sin(a * 3 + ph);
            const xx = c.cx + c.rx * grow * Math.cos(a), yy = c.cy + st.dy + c.ry * grow * Math.sin(a);
            k ? ctx.lineTo(xx, yy) : ctx.moveTo(xx, yy);
          }
          ctx.stroke();
        } else if (ul.style === 'marker') {
          const x0 = g.x0 - L.over, y = g.baseline + ul.offset + st.dy;
          const amp = th * 0.28, tilt = (rnd() - 0.5) * th * 1.2, ph = rnd() * 6.283, per = 160 + rnd() * 120;
          ctx.beginPath();
          const n = Math.max(1, Math.ceil(take / 4));
          for (let k = 0; k <= n; k++) {
            const d = Math.min(take, k * 4);
            const yy = y + Math.sin(ph + d / per * 6.283) * amp + tilt * (d / lens[gi]);
            k ? ctx.lineTo(x0 + d, yy) : ctx.moveTo(x0 + d, yy);
          }
          ctx.lineWidth = th * 1.15; ctx.globalAlpha = 0.95; ctx.stroke();
          ctx.globalAlpha = 1;
        } else {
          const y = (ul.style === 'strike' ? g.mid : g.baseline + ul.offset) + st.dy;
          const x0 = g.x0 + th / 2;
          ctx.beginPath(); ctx.moveTo(x0, y); ctx.lineTo(x0 + take, y); ctx.stroke();
        }
      });
    }
  }

  // Box outlines while editing a screenshot
  if (edit && L.mode === 'shot') {
    const lw = Math.max(2, L.minSide * 0.0025);
    ctx.lineWidth = lw; ctx.setLineDash([lw * 4, lw * 3]);
    ctx.font = `700 ${Math.round(L.minSide * 0.022)}px Barlow, Arial, sans-serif`;
    L.spans.forEach((sp, i) => sp.segs.forEach((g, gi) => {
      ctx.strokeStyle = 'rgba(244,196,67,0.95)';
      ctx.strokeRect(g.x0, g.top, g.w, g.bottom - g.top);
      if (gi === 0) {
        const label = String(i + 1), pw = L.minSide * 0.032;
        ctx.setLineDash([]); ctx.fillStyle = '#f4c443';
        roundRect(ctx, g.x0 - pw * 0.2, g.top - pw * 1.05, pw, pw * 0.9, pw * 0.2); ctx.fill();
        ctx.fillStyle = '#1d1600'; ctx.textAlign = 'center'; ctx.fillText(label, g.x0 + pw * 0.3, g.top - pw * 0.33); ctx.textAlign = 'left';
        ctx.setLineDash([lw * 4, lw * 3]);
      }
    }));
    if (shot.drag && L.img) {
      const d = shot.drag, sc = L.img.sc;
      const x = L.img.x + Math.min(d.x0, d.x1) * sc, y = L.img.y + Math.min(d.y0, d.y1) * sc;
      ctx.setLineDash([]); ctx.strokeStyle = '#ffffff'; ctx.fillStyle = 'rgba(244,196,67,0.18)';
      ctx.fillRect(x, y, Math.abs(d.x1 - d.x0) * sc, Math.abs(d.y1 - d.y0) * sc);
      ctx.strokeRect(x, y, Math.abs(d.x1 - d.x0) * sc, Math.abs(d.y1 - d.y0) * sc);
    }
    ctx.setLineDash([]);
  }
  ctx.restore();
}

/* ----- underline UI ----- */
const toolUl = $('#tool-ul');
const ulCanvas = $('#ul-canvas'), ulGuides = $('#ul-guides'), ulViewer = $('#ul-viewer'), ulMonitor = $('#ul-monitor');
function ulPaint(t) {
  if (!ulL) return;
  const { W, H } = ulL;
  if (ulCanvas.width !== W || ulCanvas.height !== H) { ulCanvas.width = W; ulCanvas.height = H; }
  const ctx = ulCanvas.getContext('2d');
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  const editing = ul.mode === 'shot' && shot.edit;
  ulDraw(ctx, ulL, ulTL, t, { fillBg: !ul.transparent, edit: editing });
  ulViewer.classList.toggle('checker', !!ul.transparent);
  ulViewer.classList.toggle('editing', editing);
  const g = ulGuides.getContext('2d');
  const dpr = window.devicePixelRatio || 1;
  const gw = Math.round(ulViewer.clientWidth * dpr), gh = Math.round(ulViewer.clientHeight * dpr);
  if (ulGuides.width !== gw || ulGuides.height !== gh) { ulGuides.width = gw; ulGuides.height = gh; }
  g.clearRect(0, 0, gw, gh);
  if (ulPlayer && ulPlayer.guides) drawGuides(g, gw, gh, ul.out.aspect, gw / W * 2);
}
function ulRefresh() {
  toolUl.dataset.mode = ul.mode;
  ulL = ulLayout(measureCtx);
  ulTL = ulTimeline(ulL);
  const { OW, OH } = dims(ul.out);
  $('#ul-fmt').textContent = `${OW} × ${OH} · ${ul.out.aspect} · ${ul.out.fps} fps`;
  const fitNote = ul.mode !== 'text' ? '' : [
    ulL.size < ul.size ? `Text shrunk to ${ulL.size}px to fit` : '',
    ulL.srcShrunk ? `Source shrunk to ${ulL.srcShrunk}px to fit` : '',
  ].filter(Boolean).join(' · ');
  const editNote = ul.mode === 'shot' && shot.edit ? 'Drawing boxes: drag over the words' : '';
  $('#ul-fit').textContent = editNote || fitNote;
  $('#ul-fit').classList.toggle('live', !!editNote);
  $('#ul-outnote').textContent = ul.transparent && ul.out.format === 'mp4'
    ? 'MP4 can’t store transparency, so the video uses your background color. Choose PNG frames to keep it transparent.'
    : ul.out.format === 'png-seq' ? 'PNG frames import into Premiere, Resolve, After Effects, and Final Cut as an image sequence.' : '';
  ulRenderTrack();
  if (ulPlayer.t > ulTL.total) ulPlayer.seek(ulTL.total); else { ulPaint(ulPlayer.t); ulPlayer.sync(); }
  $('#ul-sample-tag').hidden = ul.text !== UL_SAMPLE;
  saveSoon('ul', ul);
}
function ulRenderTrack() {
  const tr = $('#ul-track'), T = ulTL.total;
  let html = '';
  if (ulTL.introDur) html += `<div class="blk dim" style="left:0;width:${ulTL.introDur / T * 100}%;--c:#8a857c"><span>In</span></div>`;
  ulTL.items.forEach((it, i) => {
    html += `<div class="blk" style="left:${it.t0 / T * 100}%;width:${(it.t1 - it.t0) / T * 100}%;--c:${it.ref.color || ul.color}"><span>${i + 1}</span></div>`;
  });
  if (T > ulTL.drawEnd) html += `<div class="blk dim" style="left:${ulTL.drawEnd / T * 100}%;width:${(T - ulTL.drawEnd) / T * 100}%;--c:#8a857c"><span>Hold</span></div>`;
  tr.innerHTML = html + '<div class="head"></div>';
}

/* word picker */
function ulRenderWords() {
  const box = $('#ul-words');
  box.textContent = '';
  const re = /\S+|\s+/g; let m;
  while ((m = re.exec(ul.text))) {
    const tok = m[0];
    if (/^\s+$/.test(tok)) {
      tok.split('\n').forEach((part, i) => { if (i) box.appendChild(document.createElement('br')); if (part) box.appendChild(document.createTextNode(' ')); });
      continue;
    }
    const el = document.createElement('span');
    el.className = 'w'; el.textContent = tok; el.dataset.s = m.index; el.dataset.e = m.index + tok.length;
    box.appendChild(el);
  }
  ulMarkWords();
}
function ulMarkWords(dragRange) {
  $$('#ul-words .w').forEach(w => {
    const s = +w.dataset.s, e = +w.dataset.e;
    const sp = ul.spans.find(x => s < x.end && e > x.start);
    w.classList.toggle('on', !!sp);
    if (sp) w.style.setProperty('--c', sp.color || ul.color); else w.style.removeProperty('--c');
    w.classList.toggle('drag', !!dragRange && s >= dragRange[0] && e <= dragRange[1]);
  });
}
function ulRenderSpans() {
  const list = $('#ul-spans');
  const sorted = ul.spans.slice().sort((a, b) => a.start - b.start);
  list.innerHTML = '';
  sorted.forEach((sp, i) => {
    const li = document.createElement('li');
    li.innerHTML = `<input type="color" aria-label="Color for mark ${i + 1}"><span class="t"><span class="n">${i + 1}</span></span><button class="x" type="button" aria-label="Remove mark ${i + 1}">×</button>`;
    const c = li.querySelector('input'); c.value = (sp.color || ul.color).toLowerCase();
    li.querySelector('.t').append(document.createTextNode(ul.text.slice(sp.start, sp.end)));
    c.addEventListener('input', () => { sp.color = c.value; ulMarkWords(); ulRefresh(); });
    li.querySelector('.x').addEventListener('click', () => { ul.spans.splice(ul.spans.indexOf(sp), 1); ulSpansChanged(); });
    list.appendChild(li);
  });
}
function ulSpansChanged() { ulRenderSpans(); ulMarkWords(); ulRefresh(); }
const PUNCT_TRAIL = /[\s,.;:!?"'”’)\]]/, PUNCT_LEAD = /[\s"'“‘(\[]/;
function trimRange(s, e) {
  while (e > s + 1 && PUNCT_TRAIL.test(ul.text[e - 1])) e--;
  while (s < e - 1 && PUNCT_LEAD.test(ul.text[s])) s++;
  return [s, e];
}
function addSpan(s, e) {
  ul.spans = ul.spans.filter(x => !(s < x.end && e > x.start));
  const sp = { start: s, end: e, color: null };
  ul.spans.push(sp); lastAdded = sp;
  return sp;
}
function toggleWord(s, e) {
  const hit = ul.spans.find(x => s < x.end && e > x.start);
  if (hit) { ul.spans.splice(ul.spans.indexOf(hit), 1); if (lastAdded === hit) lastAdded = null; return; }
  [s, e] = trimRange(s, e);
  if (lastAdded && ul.spans.includes(lastAdded)) {
    const gapAfter = ul.text.slice(lastAdded.end, s);
    const gapBefore = ul.text.slice(e, lastAdded.start);
    if (s >= lastAdded.end && /^[\s,.;:!?"'”’)\]]*$/.test(gapAfter) && gapAfter.length <= 3) { lastAdded.end = e; return; }
    if (e <= lastAdded.start && /^[\s"'“‘(\[]*$/.test(gapBefore) && gapBefore.length <= 3) { lastAdded.start = s; return; }
  }
  addSpan(s, e);
}
(function wordPicker() {
  const box = $('#ul-words');
  let drag = null;
  const range = () => [Math.min(+drag.a.dataset.s, +drag.b.dataset.s), Math.max(+drag.a.dataset.e, +drag.b.dataset.e)];
  box.addEventListener('pointerdown', e => {
    const w = e.target.closest('.w'); if (!w) return;
    e.preventDefault();
    drag = { a: w, b: w, moved: false };
    try { box.setPointerCapture(e.pointerId); } catch (_) { /* ignore */ }
    ulMarkWords(range());
  });
  box.addEventListener('pointermove', e => {
    if (!drag) return;
    const el = document.elementFromPoint(e.clientX, e.clientY);
    const w = el && el.closest ? el.closest('#ul-words .w') : null;
    if (w && w !== drag.b) { drag.b = w; drag.moved = true; ulMarkWords(range()); }
  });
  box.addEventListener('pointerup', () => {
    if (!drag) return;
    const [s, e] = range();
    if (drag.moved) { const [a, b] = trimRange(s, e); addSpan(a, b); } else toggleWord(s, e);
    drag = null;
    ulSpansChanged();
  });
  box.addEventListener('pointercancel', () => { drag = null; ulMarkWords(); });
})();

/* keep underlines attached to the same words while the text is edited */
function remapSpans(oldText, newText) {
  ul.spans = ul.spans.map(sp => {
    const str = oldText.slice(sp.start, sp.end);
    if (!str.trim()) return null;
    let best = -1, bestD = Infinity, i = newText.indexOf(str);
    while (i !== -1) { const d = Math.abs(i - sp.start); if (d < bestD) { best = i; bestD = d; } i = newText.indexOf(str, i + 1); }
    return best >= 0 ? { ...sp, start: best, end: best + str.length } : null;
  }).filter(Boolean);
  lastAdded = null;
}
function setUlText(text, { keepSpans = true } = {}) {
  const old = ul.text;
  ul.text = text;
  if (keepSpans) remapSpans(old, text); else { ul.spans = []; lastAdded = null; }
  $('#ul-text').value = text;
  ulRenderWords(); ulRenderSpans();
  ulEnsureFonts().then(ulRefresh);
  ulRefresh();
}

/* article link */
function mdToParas(md) {
  return md.split(/\n\s*\n/).map(b => b
    .replace(/!\[[^\]]*\]\([^)]*\)/g, '')
    .replace(/\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/^\s{0,3}(#{1,6}|[-*+]|\d+\.)\s+/gm, '')
    .replace(/(\*\*|__|`|^>\s?)/gm, '')
    .replace(/\s+/g, ' ').trim())
    .filter(p => p.length >= 40 && (p.match(/\s/g) || []).length >= 6 && !/^(advertisement|subscribe|sign up|share this|read more|related:|image:|photo:)/i.test(p) && !/https?:\/\//.test(p))
    .slice(0, 40);
}
function prettyDate(s) {
  const d = new Date(s);
  return isNaN(d) ? '' : d.toLocaleDateString('en-US', { month: 'long', day: 'numeric', year: 'numeric' });
}
async function fetchArticle(url) {
  let u;
  try { u = new URL(url); } catch (e) { throw new Error('That doesn’t look like a full link. It should start with https://'); }
  const r = await fetch('https://r.jina.ai/' + u.href, { headers: { Accept: 'application/json' } });
  if (!r.ok) throw new Error(r.status === 429 ? 'The reader service is busy. Wait a minute and try again, or paste the text.' : `The page couldn’t be read (error ${r.status}). Paste the text instead.`);
  let title = '', content = '', published = '';
  const ct = r.headers.get('content-type') || '';
  if (ct.includes('json')) {
    const j = await r.json(); const d = j.data || j;
    title = d.title || ''; content = d.content || d.text || ''; published = d.publishedTime || d.published_time || '';
  } else {
    const txt = await r.text();
    title = (txt.match(/^Title:\s*(.+)$/m) || [])[1] || '';
    published = (txt.match(/^Published Time:\s*(.+)$/m) || [])[1] || '';
    const i = txt.indexOf('Markdown Content:'); content = i >= 0 ? txt.slice(i + 17) : txt;
  }
  return { title: title.trim(), paras: mdToParas(content), published, host: u.hostname.replace(/^www\./, '') };
}
$('#ul-fetch').addEventListener('click', async () => {
  const url = $('#ul-url').value.trim();
  const st = $('#ul-fetch-status'), list = $('#ul-article');
  if (!url) { st.textContent = 'Paste an article link first.'; st.className = 'status err'; return; }
  st.textContent = 'Reading the page…'; st.className = 'status'; list.innerHTML = '';
  $('#ul-fetch').disabled = true;
  try {
    const a = await fetchArticle(url);
    if (!a.title && !a.paras.length) throw new Error('No readable text came back. Paste the text instead.');
    st.textContent = `Pick the headline or a paragraph from ${a.host}.`; st.className = 'status ok';
    const items = [];
    if (a.title) items.push({ kind: 'Headline', text: a.title });
    a.paras.forEach((p, i) => items.push({ kind: `Paragraph ${i + 1}`, text: p }));
    items.forEach(it => {
      const b = document.createElement('button'); b.type = 'button';
      b.innerHTML = `<span class="kind">${it.kind}</span>`;
      b.append(document.createTextNode(it.text.length > 220 ? it.text.slice(0, 220) + '…' : it.text));
      b.addEventListener('click', () => {
        setUlText(it.text, { keepSpans: false });
        const date = prettyDate(a.published);
        bind.ul.source.set([a.host, date].filter(Boolean).join(' · '));
        ulRefresh();
        toast('Text added. Now tap the words to mark.');
      });
      list.appendChild(b);
    });
  } catch (e) {
    st.textContent = e.message && !/Failed to fetch|NetworkError|Load failed/i.test(e.message) ? e.message : 'Couldn’t reach that page. Copy the headline or paragraph and paste it instead.';
    st.className = 'status err';
  } finally { $('#ul-fetch').disabled = false; }
});

/* source mode switch */
function ulSetSrc(v) {
  ul.src = v;
  $$('input[name="ul-src"]').forEach(r => { r.checked = r.value === v; });
  ul.mode = v === 'shot' ? 'shot' : 'text';
  $('#ul-link-pane').hidden = v !== 'link';
  $('#ul-paste-pane').hidden = v === 'shot';
  if (ul.mode === 'shot' && !shot.img) shot.edit = false;
  ulStyleFields(); ulShotUI();
  ulPlayer.pause();
  ulRefresh();
}
$$('input[name="ul-src"]').forEach(r => r.addEventListener('change', () => { if (r.checked) ulSetSrc(r.value); }));

/* screenshot mode */
function ulShotUI() {
  $('#ul-shot-name').textContent = shot.img ? `${shot.name} · ${shot.img.naturalWidth} × ${shot.img.naturalHeight}` : 'No screenshot yet. You can also drop an image here or paste one (Ctrl/Cmd+V).';
  $('#ul-shot-edit').setAttribute('aria-pressed', String(shot.edit));
  $('#ul-shot-edit').disabled = !shot.img;
  const list = $('#ul-marks');
  list.innerHTML = '';
  shot.marks.forEach((mk, i) => {
    const li = document.createElement('li');
    li.innerHTML = `<input type="color" aria-label="Color for mark ${i + 1}"><span class="t"><span class="n">${i + 1}</span>${mk.boxes.length === 1 ? '1 box' : mk.boxes.length + ' boxes'}</span><button class="x" type="button" aria-label="Remove mark ${i + 1}">×</button>`;
    const c = li.querySelector('input'); c.value = (mk.color || ul.color).toLowerCase();
    c.addEventListener('input', () => { mk.color = c.value; ulRefresh(); });
    li.querySelector('.x').addEventListener('click', () => { shot.marks.splice(i, 1); ulShotUI(); ulRefresh(); });
    list.appendChild(li);
  });
}
async function ulLoadShot(file) {
  if (!file || !/^image\//.test(file.type)) { toast('That isn’t an image file.'); return; }
  try {
    const img = await loadImageFile(file);
    shot.img = img; shot.name = file.name || 'Pasted image'; shot.marks = []; shot.edit = true;
    if (ul.src !== 'shot') ulSetSrc('shot');
    ulShotUI(); ulRefresh();
    toast('Screenshot added. Drag across the words you want to mark.');
  } catch (e) { toast(e.message); }
}
$('#ul-shot-pick').addEventListener('click', () => $('#ul-shot-file').click());
$('#ul-shot-file').addEventListener('change', e => { ulLoadShot(e.target.files[0]); e.target.value = ''; });
$('#ul-shot-edit').addEventListener('click', () => { shot.edit = !shot.edit; ulPlayer.pause(); ulShotUI(); ulRefresh(); });
$('#ul-marks-clear').addEventListener('click', () => { shot.marks = []; ulShotUI(); ulRefresh(); });
const shotJoin = { join: false };
bindCheck('ul-shot-join', shotJoin, 'join', () => {});
[$('#ul-drop'), ulMonitor].forEach(el => {
  el.addEventListener('dragover', e => { if ([...e.dataTransfer.items].some(i => i.kind === 'file')) { e.preventDefault(); $('#ul-drop').classList.add('over'); } });
  el.addEventListener('dragleave', () => $('#ul-drop').classList.remove('over'));
  el.addEventListener('drop', e => { e.preventDefault(); $('#ul-drop').classList.remove('over'); const f = e.dataTransfer.files[0]; if (f) ulLoadShot(f); });
});
document.addEventListener('paste', e => {
  if ($('#tool-ul').hidden || e.target?.closest?.('input, textarea')) return;
  const item = [...(e.clipboardData?.items || [])].find(i => i.type.startsWith('image/'));
  if (item) { e.preventDefault(); ulLoadShot(item.getAsFile()); }
});
function toImageCoords(e) {
  const r = ulCanvas.getBoundingClientRect();
  const fx = (e.clientX - r.left) / r.width * ulL.W, fy = (e.clientY - r.top) / r.height * ulL.H;
  return { x: clamp((fx - ulL.img.x) / ulL.img.sc, 0, ulL.img.iw), y: clamp((fy - ulL.img.y) / ulL.img.sc, 0, ulL.img.ih) };
}
ulCanvas.addEventListener('pointerdown', e => {
  if (ul.mode !== 'shot' || !shot.edit || !ulL?.img) return;
  e.preventDefault();
  try { ulCanvas.setPointerCapture(e.pointerId); } catch (_) { /* ignore */ }
  const p = toImageCoords(e);
  shot.drag = { x0: p.x, y0: p.y, x1: p.x, y1: p.y };
  ulPaint(ulPlayer.t);
});
ulCanvas.addEventListener('pointermove', e => {
  if (!shot.drag) return;
  const p = toImageCoords(e); shot.drag.x1 = p.x; shot.drag.y1 = p.y;
  ulPaint(ulPlayer.t);
});
ulCanvas.addEventListener('pointerup', () => {
  const d = shot.drag; shot.drag = null;
  if (!d) return;
  const box = { x: Math.min(d.x0, d.x1), y: Math.min(d.y0, d.y1), w: Math.abs(d.x1 - d.x0), h: Math.abs(d.y1 - d.y0) };
  const minPx = 6 / ulL.img.sc;
  if (box.w < minPx || box.h < minPx) { ulPaint(ulPlayer.t); return; }
  if (shotJoin.join && shot.marks.length) shot.marks[shot.marks.length - 1].boxes.push(box);
  else shot.marks.push({ boxes: [box], color: null });
  ulShotUI(); ulRefresh();
});
ulCanvas.addEventListener('pointercancel', () => { shot.drag = null; ulPaint(ulPlayer.t); });

/* headshot */
$('#ul-headshot-pick').addEventListener('click', () => $('#ul-headshot-file').click());
$('#ul-headshot-file').addEventListener('change', async e => {
  const f = e.target.files[0]; e.target.value = '';
  if (!f) return;
  try {
    headshot = await loadImageFile(f);
    $('#ul-headshot-img').src = headshot.src; $('#ul-headshot-img').hidden = false; $('#ul-headshot-clear').hidden = false;
    ulRefresh();
  } catch (err) { toast(err.message); }
});
$('#ul-headshot-clear').addEventListener('click', () => { headshot = null; $('#ul-headshot-img').hidden = true; $('#ul-headshot-clear').hidden = true; ulRefresh(); });

/* background image */
function ulBgImgUI() {
  $('#ul-bgimg-name').textContent = bgImg ? bgImg.dataset.name : 'None. Choose a photo or logo to fill the frame or repeat as a grid.';
  $('#ul-bgimg-clear').hidden = !bgImg;
  $('#ul-bgimg-f').hidden = !bgImg;
  $('#ul-bggrid-f').hidden = ul.bgFit !== 'grid';
}
$('#ul-bgimg-pick').addEventListener('click', () => $('#ul-bgimg-file').click());
$('#ul-bgimg-file').addEventListener('change', async e => {
  const f = e.target.files[0]; e.target.value = '';
  if (!f) return;
  try {
    bgImg = await loadImageFile(f); bgImg.dataset.name = f.name;
    if (ul.transparent) { bind.ul.transparent.set(false); toast('Turned off the transparent background so the image shows.'); }
    ulBgImgUI(); ulRefresh();
  } catch (err) { toast(err.message); }
});
$('#ul-bgimg-clear').addEventListener('click', () => { if (bgImg?.src.startsWith('blob:')) URL.revokeObjectURL(bgImg.src); bgImg = null; ulBgImgUI(); ulRefresh(); });

/* font select */
function buildFontSelect() {
  const sel = $('#ul-font');
  const groups = [...new Set(FONTS.map(f => f.group))];
  sel.innerHTML = groups.map(g => `<optgroup label="${g}">${FONTS.filter(f => f.group === g).map(f => `<option value="${f.name}">${f.name}</option>`).join('')}</optgroup>`).join('');
  sel.value = ul.font;
}
$('#ul-upload').addEventListener('click', () => $('#ul-fontfile').click());
$('#ul-fontfile').addEventListener('change', async e => {
  const file = e.target.files[0]; if (!file) return;
  try {
    const name = 'Uploaded ' + file.name.replace(/\.[^.]+$/, '').replace(/[^\w -]/g, '').slice(0, 40);
    const face = new FontFace(name, await file.arrayBuffer());
    await face.load(); document.fonts.add(face);
    if (!FONTS.some(f => f.name === name)) FONTS.push({ name, group: 'Uploaded (this session)', stack: `"${name}", Arial, sans-serif` });
    ul.font = name; ul.customFont = ''; $('#ul-customfont').value = '';
    buildFontSelect(); ulRefresh();
    toast(`Using ${file.name}. Uploaded fonts need re-uploading after you reopen the file.`);
  } catch (err) { toast('That font file couldn’t be read. Try a .ttf, .otf, or .woff2 file.'); }
  e.target.value = '';
});

/* bindings */
const bind = { ul: {} };
const ulChanged = key => {
  if (key === 'font' || key === 'weight' || key === 'customFont') ulEnsureFonts().then(ulRefresh);
  if (key === 'color') { ulMarkWords(); ulRenderSpans(); ulShotUI(); }
  if (key === 'out.aspect') ulFit();
  ulStyleFields();
  ulRefresh();
};
function ulStyleFields() {
  const st = ul.style;
  $('#ul-thick-f').hidden = st === 'highlight';
  $('#ul-offset-f').hidden = !(st === 'underline' || st === 'marker');
  $('#ul-hlh-f').hidden = st !== 'highlight';
  $('#ul-hlo-f').hidden = st !== 'highlight';
  $('#ul-hltext-f').hidden = st !== 'highlight';
  $('#ul-hltext-c').hidden = !ul.hlTextOn;
  $('#ul-cardcolor-f').hidden = !ul.card;
  $('#ul-quote-f').hidden = !ul.quote;
  $('#ul-push-f').hidden = ul.camera !== 'push';
  $('#ul-follow-f').hidden = ul.camera !== 'follow';
  $('#ul-wordgap-f').hidden = ul.intro !== 'words';
  $('#ul-font').value = ul.font;
  ulBgImgUI();
}
buildFontSelect();
$('#ul-text').value = ul.text;
$('#ul-text').addEventListener('input', e => setUlText(e.target.value));
bind.ul.source = bindText('ul-source', ul, 'source', ulChanged);
bindCheck('ul-quote', ul, 'quote', ulChanged);
bindText('ul-qname', ul, 'quoteName', ulChanged);
bindText('ul-qtitle', ul, 'quoteTitle', ulChanged);
bindSeg('ul-style', ul, 'style', ulChanged);
bindColor('ul-color', ul, 'color', ulChanged);
bindRange('ul-thick', ul, 'thick', v => v + 'px', ulChanged);
bindRange('ul-offset', ul, 'offset', v => v + 'px', ulChanged);
bindRange('ul-hlh', ul, 'hlHeight', v => v + '%', ulChanged);
bindRange('ul-hlo', ul, 'hlOpacity', v => Math.round(v * 100) + '%', ulChanged);
bindCheck('ul-hltext-on', ul, 'hlTextOn', ulChanged);
bindColor('ul-hltext', ul, 'hlText', ulChanged);
bindColor('ul-bg', ul, 'bg', ulChanged);
bind.ul.transparent = bindCheck('ul-transparent', ul, 'transparent', ulChanged);
bindSeg('ul-bgfit', ul, 'bgFit', ulChanged);
bindSeg('ul-bggrid', ul, 'bgGrid', ulChanged);
bindRange('ul-bgcols', ul, 'bgCols', v => String(v), ulChanged);
bindRange('ul-bggap', ul, 'bgGap', v => v + 'px', ulChanged);
bindRange('ul-bgdim', ul, 'bgDim', v => Math.round(v * 100) + '%', ulChanged);
bindRange('ul-bgblur', ul, 'bgBlur', v => v + 'px', ulChanged);
bindCheck('ul-bgdrift', ul, 'bgDrift', ulChanged);
bindColor('ul-fg', ul, 'fg', ulChanged);
bindColor('ul-srccolor', ul, 'srcColor', ulChanged);
bindCheck('ul-card', ul, 'card', ulChanged);
bindColor('ul-cardcolor', ul, 'cardColor', ulChanged);
bindSeg('ul-shotfit', ul, 'shotFit', ulChanged);
bindSelect('ul-font', ul, 'font', ulChanged);
bindSelect('ul-weight', ul, 'weight', ulChanged);
bindText('ul-customfont', ul, 'customFont', ulChanged);
bindRange('ul-size', ul, 'size', v => v + 'px', ulChanged);
bindRange('ul-srcsize', ul, 'srcSize', v => v + 'px', ulChanged);
bindCheck('ul-autofit', ul, 'autoFit', ulChanged);
bindRange('ul-lh', ul, 'lineHeight', v => (+v).toFixed(2), ulChanged);
bindSeg('ul-align', ul, 'align', ulChanged);
bindSeg('ul-valign', ul, 'valign', ulChanged);
bindSeg('ul-intro', ul, 'intro', ulChanged);
bindRange('ul-wordgap', ul, 'wordGap', fmtSec, ulChanged);
bindRange('ul-draw', ul, 'draw', fmtSec, ulChanged);
bindCheck('ul-matchspeed', ul, 'matchSpeed', ulChanged);
bindRange('ul-delay', ul, 'delay', fmtSec, ulChanged);
bindRange('ul-gap', ul, 'gap', fmtSec, ulChanged);
bindRange('ul-hold', ul, 'hold', fmtSec, ulChanged);
bindSeg('ul-order', ul, 'order', ulChanged);
bindSeg('ul-ease', ul, 'ease', ulChanged);
bindSeg('ul-camera', ul, 'camera', ulChanged);
bindRange('ul-push', ul, 'push', v => v + '%', ulChanged);
bindRange('ul-followmax', ul, 'followMax', v => (+v).toFixed(1) + '×', ulChanged);
bindCheck('ul-followreturn', ul, 'followReturn', ulChanged);

$('#ul-clear').addEventListener('click', () => { ul.spans = []; lastAdded = null; ulSpansChanged(); });
function addPhrase() {
  const q = $('#ul-find').value.trim(); if (!q) return;
  const hay = ul.text.toLowerCase(), needle = q.toLowerCase();
  let i = hay.indexOf(needle);
  while (i !== -1 && ul.spans.some(x => i < x.end && i + needle.length > x.start)) i = hay.indexOf(needle, i + 1);
  if (i === -1) { toast(hay.includes(needle) ? 'That phrase is already marked.' : `“${q}” isn’t in the text. Check spelling and punctuation.`); return; }
  addSpan(i, i + needle.length); $('#ul-find').value = ''; ulSpansChanged();
}
$('#ul-find-add').addEventListener('click', addPhrase);
$('#ul-find').addEventListener('keydown', e => { if (e.key === 'Enter') { e.preventDefault(); addPhrase(); } });

const ulPlayer = makePlayer('ul', {
  total: () => (ulTL ? ulTL.total : 1), fps: () => ul.out.fps, render: t => ulPaint(t), track: $('#ul-track'),
  beforePlay: async () => { if (shot.edit) { shot.edit = false; ulShotUI(); ulRefresh(); } return true; },
});

async function ulRenderOne() {
  await ulEnsureFonts();
  shot.edit = false; ulShotUI();
  ulRefresh();
  ulPlayer.pause();
  if (ul.mode === 'shot' && !shot.img) { toast('Choose a screenshot first.'); return 'error'; }
  const L = ulL, TL = ulTL, { q, OW, OH } = dims(ul.out);
  const fmt = ul.out.format;
  const keepAlpha = fmt === 'png-seq' && ul.transparent;
  const label = ul.mode === 'shot' ? shot.name.replace(/\.[^.]+$/, '') : ul.text.slice(0, 40);
  return runRender({
    title: `Rendering ${ul.out.aspect}`, OW, OH, fps: ul.out.fps, total: TL.total, format: fmt,
    name: `underline_${slug(label)}_${OW}x${OH}`,
    frame: (ctx, t) => { ctx.setTransform(q, 0, 0, q, 0, 0); ulDraw(ctx, L, TL, t, { fillBg: !keepAlpha }); },
  });
}
async function ulStill() {
  await ulEnsureFonts(); ulRefresh();
  const { q, OW, OH } = dims(ul.out);
  const c = document.createElement('canvas'); c.width = OW; c.height = OH;
  const ctx = c.getContext('2d'); ctx.setTransform(q, 0, 0, q, 0, 0);
  ulDraw(ctx, ulL, ulTL, ulPlayer.t, { fillBg: !ul.transparent });
  saveStill(c, `underline_frame_${OW}x${OH}.png`);
}
buildOutputPanel('ul', ul.out, ulChanged, {
  render: ulRenderOne,
  renderBoth: () => renderBoth(ul.out, () => { ulFit(); ulRefresh(); }, ulRenderOne),
  still: ulStill,
});
buildPresetPanel('ul', ul, UL_PRESET_KEYS, UL_BUILTINS, () => { ulStyleFields(); ulEnsureFonts().then(ulRefresh); ulFit(); ulRefresh(); });

function ulFit() { fitViewer(ulMonitor, ulViewer, ul.out.aspect); ulPaint(ulPlayer.t); }
new ResizeObserver(() => { if (!$('#tool-ul').hidden) ulFit(); }).observe(ulMonitor);

CS.tools.ul = {
  id: 'ul',
  player: ulPlayer,
  boot() {
    ul.mode = ul.src === 'shot' ? 'shot' : 'text';
    ulSetSrc(ul.src);
    ulRenderWords(); ulRenderSpans(); ulShotUI(); ulStyleFields();
    ulRefresh();
    ulPlayer.seek(Math.max(0, ulTL.drawEnd + Math.min(0.3, ul.hold)));
    ulEnsureFonts().then(() => { ulRefresh(); });
    if (document.fonts && document.fonts.addEventListener) document.fonts.addEventListener('loadingdone', () => ulRefresh());
  },
  show() { requestAnimationFrame(ulFit); },
  hide() { ulPlayer.pause(); },
  debug: { ul, shot, ulPlayer, get ulBgImg() { return bgImg; }, get ulTL() { return ulTL; }, get ulL() { return ulL; } },
};
})();
