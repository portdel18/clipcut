/* Clip Studio · Numbers tool (counters and simple animated charts)
   Classic script (not an ES module) so index.html still works when opened straight from disk.
   Shared helpers live on window.CS; each tool registers itself in CS.tools.

   Data is typed as "Label | Value" rows. A value keeps the formatting it was typed with: "$2.4 billion" counts from
   "$0.0 billion" to "$2.4 billion", "1,040,000" keeps its commas. A "*" before a label highlights that row.
   Counter and Ring show the first row; Bars and Line use every row. nmLayout → nmTimeline → nmDraw. */
(() => {
'use strict';
const {
  $, clamp, lerp, EASE, ease, easeOutBack, fmtSec, slug, clone, store, toast, roundRect,
  FRAMES, dims, resync, saveSoon, bindRange, bindColor, bindCheck, bindSeg, bindSelect,
  buildOutputPanel, buildPresetPanel, makePlayer, fitViewer, drawGuides, runRender, renderBoth, saveStill,
} = window.CS;

const FONTS = {
  'Libre Franklin': '"Libre Franklin", Arial, sans-serif',
  'Inter': 'Inter, Arial, sans-serif',
  'Oswald': 'Oswald, "Arial Narrow", sans-serif',
  'Source Serif 4': '"Source Serif 4", Georgia, serif',
};
const NM_SAMPLE = ['City A | 412,000', '*City B | 298,000', 'City C | 176,000', 'City D | 94,500'].join('\n');
const NM_DEFAULT = {
  kind: 'bars', data: NM_SAMPLE, title: 'Sample: lead service lines still in use', source: 'Sample data',
  zero: true, sortBars: false,
  bg: '#0F1216', fg: '#F5F2EC', muted: '#8C96A1', accent: '#F4C443', barColor: '#4A5563',
  font: 'Libre Franklin', ease: 'smooth',
  delay: 0.4, build: 1.8, stagger: 0.12, hold: 2,
  out: { aspect: '16:9', quality: '1080', fps: 30, format: 'mp4' },
};
const NM_PRESET_KEYS = ['kind', 'zero', 'sortBars', 'bg', 'fg', 'muted', 'accent', 'barColor', 'font', 'ease', 'delay', 'build', 'stagger', 'hold', 'out'];
const NM_BUILTINS = {
  'Night desk bars': { kind: 'bars', bg: '#0F1216', fg: '#F5F2EC', muted: '#8C96A1', accent: '#F4C443', barColor: '#4A5563', font: 'Libre Franklin', ease: 'smooth', build: 1.8, stagger: 0.12 },
  'Big number': { kind: 'counter', bg: '#0B0B0C', fg: '#FFFFFF', muted: '#A1A1A6', accent: '#FF3B30', font: 'Oswald', ease: 'glide', build: 2.2 },
  'Paper line chart': { kind: 'line', zero: false, bg: '#EFEBE3', fg: '#161514', muted: '#6B665F', accent: '#C8102E', barColor: '#B9B2A6', font: 'Source Serif 4', ease: 'smooth', build: 2.4 },
  'Percent ring (9:16)': { kind: 'ring', bg: '#101418', fg: '#F5F2EC', muted: '#56606B', accent: '#34C759', font: 'Inter', ease: 'glide', build: 2, out: { aspect: '9:16' } },
};

const nm = Object.assign(clone(NM_DEFAULT), store.get('nm', {}));
nm.out = Object.assign({}, NM_DEFAULT.out, nm.out || {});
if (!FRAMES[nm.out.aspect]) nm.out.aspect = '16:9';
if (!FONTS[nm.font]) nm.font = NM_DEFAULT.font;
if (!['counter', 'bars', 'line', 'ring'].includes(nm.kind)) nm.kind = 'bars';

const toolNm = $('#tool-nm');
const nmCanvas = $('#nm-canvas'), nmGuides = $('#nm-guides'), nmViewer = $('#nm-viewer'), nmMonitor = $('#nm-monitor');
const mctx = document.createElement('canvas').getContext('2d');
let nmL = null, nmTL = null;

/* ---------- data ---------- */
/** "$2.4 billion" → { prefix: '$', value: 2.4, suffix: ' billion', decimals: 1, group: false } */
function parseValue(s) {
  const m = String(s).trim().match(/^(.*?)(-?\d[\d,]*(?:\.\d+)?|-?\.\d+)(.*)$/);
  if (!m) return null;
  const num = m[2].replace(/,/g, '');
  return { prefix: m[1], value: parseFloat(num), suffix: m[3], decimals: (num.split('.')[1] || '').length, group: m[2].includes(',') };
}
const fmtValue = (v, f) => f.prefix + (f.group ? v.toLocaleString('en-US', { minimumFractionDigits: f.decimals, maximumFractionDigits: f.decimals }) : v.toFixed(f.decimals)) + f.suffix;
function parseRows(text) {
  const rows = [];
  for (const line of text.split('\n')) {
    if (!line.trim()) continue;
    const parts = line.split(/\s*\|\s*|\t/);
    const raw = parts.length > 1 ? parts.slice(1).join(' ') : parts[0];
    const label = parts.length > 1 ? parts[0].trim() : '';
    const f = parseValue(raw);
    if (!f || !isFinite(f.value)) continue;
    rows.push({ label: label.replace(/^\*\s*/, ''), hi: label.startsWith('*'), ...f });
  }
  return rows.slice(0, 24);
}

/* ---------- layout ---------- */
function nmLayout() {
  const { W, H } = dims(nm.out);
  const minSide = Math.min(W, H);
  const stack = FONTS[nm.font];
  let rows = parseRows(nm.data);
  if (nm.kind === 'bars' && nm.sortBars) rows = rows.slice().sort((a, b) => b.value - a.value);
  if (nm.kind === 'bars') rows = rows.slice(0, 12);
  const m = minSide * 0.07;
  const ts = minSide * 0.05, ss = minSide * 0.024;
  mctx.font = `700 ${ts}px ${stack}`;
  const titleLines = wrap(nm.title.trim(), W - 2 * m).slice(0, 2);
  const top = m + (titleLines.length ? titleLines.length * ts * 1.15 + ts * 0.6 : 0);
  const bottom = H - m - (nm.source.trim() ? ss * 2 : 0);
  return { W, H, minSide, stack, rows, m, ts, ss, titleLines, area: { x: m, y: top, w: W - 2 * m, h: bottom - top } };
}
function wrap(text, maxW) {
  const words = text.split(/\s+/).filter(Boolean), lines = [];
  let line = '';
  for (const w of words) {
    const next = line ? line + ' ' + w : w;
    if (mctx.measureText(next).width > maxW && line) { lines.push(line); line = w; } else line = next;
  }
  if (line) lines.push(line);
  return lines;
}
function nmTimeline(L) {
  const n = nm.kind === 'bars' ? L.rows.length : 1;
  const start = 0.5 + nm.delay;
  const build = Math.max(0.2, nm.build);
  const each = Array.from({ length: n }, (_, i) => ({ t0: start + i * nm.stagger, t1: start + i * nm.stagger + build }));
  const end = n ? each[n - 1].t1 : start;
  return { start, build, each, end, total: end + nm.hold };
}
const prog = (it, t) => ease(nm.ease, (t - it.t0) / Math.max(0.05, it.t1 - it.t0));

/* ---------- drawing ---------- */
function fitFont(ctx, text, weight, size, maxW, stack) {
  ctx.font = `${weight} ${size}px ${stack}`;
  const w = ctx.measureText(text).width;
  if (w > maxW) { size *= maxW / w; ctx.font = `${weight} ${size}px ${stack}`; }
  return size;
}
function drawCounter(ctx, L, TL, t) {
  const r = L.rows[0], A = L.area; if (!r) return;
  const p = prog(TL.each[0], t);
  const txt = fmtValue(r.value * p, r);
  const final = fmtValue(r.value, r);
  // Size from the final text so the number doesn't change size while counting
  const size = fitFont(ctx, final, 800, L.minSide * 0.22, A.w * 0.92, L.stack);
  const cy = A.y + A.h * 0.52;
  ctx.textAlign = 'center'; ctx.textBaseline = 'alphabetic';
  ctx.fillStyle = r.hi ? nm.accent : nm.fg;
  ctx.fillText(txt, A.x + A.w / 2, cy + size * 0.35);
  if (r.label) {
    const a = EASE.smooth(clamp((t - TL.start - 0.2) / 0.5, 0, 1));
    ctx.globalAlpha = a; ctx.fillStyle = nm.muted;
    fitFont(ctx, r.label, 600, L.minSide * 0.045, A.w * 0.9, L.stack);
    ctx.fillText(r.label, A.x + A.w / 2, cy + size * 0.35 + L.minSide * 0.08);
    ctx.globalAlpha = 1;
  }
  ctx.fillStyle = nm.accent;
  const bw = L.minSide * 0.12 * EASE.smooth(clamp((t - TL.start) / 0.6, 0, 1));
  ctx.fillRect(A.x + A.w / 2 - bw / 2, cy - size * 0.95, bw, Math.max(4, L.minSide * 0.008));
}
function drawBars(ctx, L, TL, t) {
  const A = L.area, rows = L.rows; if (!rows.length) return;
  const max = Math.max(...rows.map(r => Math.abs(r.value)), 1e-9);
  const n = rows.length;
  const slot = A.h / n, bh = Math.min(slot * 0.62, L.minSide * 0.09);
  const fs = clamp(bh * 0.55, L.minSide * 0.022, L.minSide * 0.045);
  ctx.font = `600 ${fs}px ${L.stack}`;
  const labelW = Math.min(A.w * 0.34, Math.max(...rows.map(r => ctx.measureText(r.label).width)) + fs);
  ctx.font = `700 ${fs}px ${L.stack}`;
  const valueW = Math.max(...rows.map(r => ctx.measureText(fmtValue(r.value, r)).width)) + fs;
  const x0 = A.x + labelW, full = Math.max(10, A.w - labelW - valueW);
  rows.forEach((r, i) => {
    const it = TL.each[i], p = prog(it, t);
    const appear = clamp((t - it.t0 + 0.15) / 0.3, 0, 1);
    if (appear <= 0) return;
    const y = A.y + slot * i + (slot - bh) / 2;
    ctx.globalAlpha = appear;
    ctx.fillStyle = nm.muted; ctx.font = `600 ${fs}px ${L.stack}`; ctx.textAlign = 'right'; ctx.textBaseline = 'middle';
    ctx.fillText(r.label, x0 - fs * 0.6, y + bh / 2, labelW - fs * 0.6);
    const w = full * Math.abs(r.value) / max * p;
    ctx.fillStyle = r.hi ? nm.accent : nm.barColor;
    roundRect(ctx, x0, y, Math.max(w, 1), bh, Math.min(bh * 0.18, 8)); ctx.fill();
    ctx.fillStyle = r.hi ? nm.accent : nm.fg; ctx.font = `700 ${fs}px ${L.stack}`; ctx.textAlign = 'left';
    ctx.fillText(fmtValue(r.value * p, r), x0 + w + fs * 0.5, y + bh / 2);
    ctx.globalAlpha = 1;
  });
}
function drawLine(ctx, L, TL, t) {
  const A = L.area, rows = L.rows; if (rows.length < 2) { drawCounter(ctx, L, TL, t); return; }
  const fs = L.minSide * 0.026;
  const vals = rows.map(r => r.value);
  let lo = nm.zero ? Math.min(0, ...vals) : Math.min(...vals), hi = Math.max(...vals);
  if (hi - lo < 1e-9) { hi += 1; lo -= nm.zero ? 0 : 1; }
  const padV = nm.zero ? 0 : (hi - lo) * 0.12; lo -= padV; hi += padV;
  ctx.font = `600 ${fs}px ${L.stack}`;
  const tick = v => fmtValue(v, { ...rows[0], decimals: Math.max(0, Math.min(2, rows[0].decimals)) });
  const yW = Math.max(ctx.measureText(tick(hi)).width, ctx.measureText(tick(lo)).width) + fs;
  const px0 = A.x + yW, px1 = A.x + A.w - fs * 4, py0 = A.y + fs, py1 = A.y + A.h - fs * 2.2;
  const X = i => lerp(px0, px1, i / (rows.length - 1)), Y = v => lerp(py1, py0, (v - lo) / (hi - lo));
  // Grid and labels fade in first
  const g = EASE.smooth(clamp((t - TL.start + 0.3) / 0.5, 0, 1));
  ctx.globalAlpha = g;
  ctx.strokeStyle = nm.barColor; ctx.lineWidth = Math.max(1, L.minSide * 0.0015);
  ctx.fillStyle = nm.muted; ctx.textAlign = 'right'; ctx.textBaseline = 'middle';
  for (let k = 0; k <= 3; k++) {
    const v = lo + (hi - lo) * k / 3, y = Y(v);
    ctx.beginPath(); ctx.moveTo(px0, y); ctx.lineTo(px1, y); ctx.stroke();
    ctx.fillText(tick(v), px0 - fs * 0.5, y);
  }
  ctx.textAlign = 'center'; ctx.textBaseline = 'top';
  const every = Math.ceil(rows.length / 8);
  rows.forEach((r, i) => { if (i % every === 0 || i === rows.length - 1) ctx.fillText(r.label, X(i), py1 + fs * 0.7); });
  ctx.globalAlpha = 1;
  // The line draws left to right; the head carries the current value
  const p = prog(TL.each[0], t);
  if (p <= 0) return;
  const head = p * (rows.length - 1), k = Math.floor(head), f = head - k;
  const hv = k >= rows.length - 1 ? vals[vals.length - 1] : lerp(vals[k], vals[k + 1], f);
  const hx = lerp(px0, px1, head / (rows.length - 1)), hy = Y(hv);
  const lw = Math.max(3, L.minSide * 0.006);
  ctx.save();
  ctx.beginPath(); ctx.moveTo(X(0), Y(vals[0]));
  for (let i = 1; i <= k; i++) ctx.lineTo(X(i), Y(vals[i]));
  ctx.lineTo(hx, hy);
  ctx.lineTo(hx, py1); ctx.lineTo(X(0), py1); ctx.closePath();
  ctx.globalAlpha = 0.14; ctx.fillStyle = nm.accent; ctx.fill();
  ctx.restore();
  ctx.beginPath(); ctx.moveTo(X(0), Y(vals[0]));
  for (let i = 1; i <= k; i++) ctx.lineTo(X(i), Y(vals[i]));
  ctx.lineTo(hx, hy);
  ctx.strokeStyle = nm.accent; ctx.lineWidth = lw; ctx.lineJoin = 'round'; ctx.lineCap = 'round'; ctx.stroke();
  rows.forEach((r, i) => { if (i <= head + 1e-6 && r.hi) { ctx.fillStyle = nm.fg; ctx.beginPath(); ctx.arc(X(i), Y(vals[i]), lw * 1.6, 0, Math.PI * 2); ctx.fill(); } });
  ctx.fillStyle = nm.accent; ctx.beginPath(); ctx.arc(hx, hy, lw * 2, 0, Math.PI * 2); ctx.fill();
  ctx.fillStyle = nm.fg; ctx.font = `700 ${fs * 1.3}px ${L.stack}`; ctx.textAlign = 'left'; ctx.textBaseline = 'middle';
  const last = rows[rows.length - 1];
  ctx.fillText(fmtValue(hv, last), Math.min(hx + lw * 3, A.x + A.w - ctx.measureText(fmtValue(hv, last)).width), hy - fs * 1.4);
}
function drawRing(ctx, L, TL, t) {
  const r = L.rows[0], A = L.area; if (!r) return;
  const pct = clamp(r.value, 0, 100) / 100;
  const p = prog(TL.each[0], t);
  const R = Math.min(A.w, A.h) * 0.36, th = R * 0.2;
  const cx = A.x + A.w / 2, cy = A.y + A.h * (r.label ? 0.44 : 0.5);
  const grow = easeOutBack(clamp((t - TL.start + 0.3) / 0.5, 0, 1));
  ctx.lineWidth = th * grow; ctx.lineCap = 'round';
  ctx.strokeStyle = nm.muted; ctx.globalAlpha = 0.35;
  ctx.beginPath(); ctx.arc(cx, cy, R, 0, Math.PI * 2); ctx.stroke();
  ctx.globalAlpha = 1;
  if (p > 0 && pct > 0) {
    ctx.strokeStyle = nm.accent;
    ctx.beginPath(); ctx.arc(cx, cy, R, -Math.PI / 2, -Math.PI / 2 + Math.PI * 2 * pct * p); ctx.stroke();
  }
  ctx.fillStyle = nm.fg; ctx.textAlign = 'center'; ctx.textBaseline = 'middle';
  fitFont(ctx, fmtValue(r.value, r), 800, R * 0.55, R * 1.4, L.stack);
  ctx.fillText(fmtValue(r.value * p, r), cx, cy + R * 0.03);
  if (r.label) {
    const a = EASE.smooth(clamp((t - TL.start - 0.2) / 0.5, 0, 1));
    ctx.globalAlpha = a; ctx.fillStyle = nm.muted;
    fitFont(ctx, r.label, 600, L.minSide * 0.042, A.w * 0.9, L.stack);
    ctx.fillText(r.label, cx, cy + R + th + L.minSide * 0.06);
    ctx.globalAlpha = 1;
  }
}
function nmDraw(ctx, W, H, t, L, TL) {
  ctx.save();
  ctx.fillStyle = nm.bg; ctx.fillRect(0, 0, W, H);
  // Title and source line
  const a = EASE.smooth(clamp(t / 0.5, 0, 1));
  ctx.globalAlpha = a; ctx.textAlign = 'left'; ctx.textBaseline = 'alphabetic';
  if (L.titleLines.length) {
    ctx.fillStyle = nm.accent; ctx.fillRect(L.m, L.m - L.ts * 0.55, L.ts * 1.4, Math.max(4, L.ts * 0.1));
    ctx.fillStyle = nm.fg; ctx.font = `700 ${L.ts}px ${L.stack}`;
    L.titleLines.forEach((ln, k) => ctx.fillText(ln, L.m, L.m + L.ts * 0.95 + k * L.ts * 1.15 - (1 - a) * L.ts * 0.3));
  }
  if (nm.source.trim()) { ctx.fillStyle = nm.muted; ctx.font = `500 ${L.ss}px ${L.stack}`; ctx.fillText(nm.source.trim(), L.m, L.H - L.m); }
  ctx.globalAlpha = 1;
  if (!L.rows.length) {
    ctx.fillStyle = nm.muted; ctx.textAlign = 'center'; ctx.font = `600 ${L.minSide * 0.035}px ${L.stack}`;
    ctx.fillText('Type a row like  Label | 42', W / 2, H / 2);
  } else ({ counter: drawCounter, bars: drawBars, line: drawLine, ring: drawRing })[nm.kind](ctx, L, TL, t);
  ctx.restore();
}

/* ---------- preview ---------- */
function nmPaint(t) {
  if (!nmL) return;
  const { W, H } = nmL;
  if (nmCanvas.width !== W || nmCanvas.height !== H) { nmCanvas.width = W; nmCanvas.height = H; }
  const ctx = nmCanvas.getContext('2d');
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  nmDraw(ctx, W, H, t, nmL, nmTL);
  const g = nmGuides.getContext('2d');
  const dpr = window.devicePixelRatio || 1;
  const gw = Math.round(nmViewer.clientWidth * dpr), gh = Math.round(nmViewer.clientHeight * dpr);
  if (nmGuides.width !== gw || nmGuides.height !== gh) { nmGuides.width = gw; nmGuides.height = gh; }
  g.clearRect(0, 0, gw, gh);
  if (nmPlayer.guides) drawGuides(g, gw, gh, nm.out.aspect, gw / W * 2);
}
function nmRenderTrack() {
  const tr = $('#nm-track'), T = nmTL.total;
  const blk = (from, len, c, label, dim) => `<div class="blk${dim ? ' dim' : ''}" style="left:${from / T * 100}%;width:${Math.max(0.3, len / T * 100)}%;--c:${c}">${label ? `<span>${label}</span>` : ''}</div>`;
  let html = blk(0, nmTL.start, '#8a857c', 'In', true);
  html += blk(nmTL.start, nmTL.end - nmTL.start, nm.accent, 'Build');
  tr.innerHTML = html + '<div class="head"></div>';
}
const USES = { counter: 'Uses the first row.', ring: 'Uses the first row, as a percent (0–100).', bars: 'Uses every row (up to 12). Put * before a label to highlight its bar.', line: 'Uses every row, left to right. Put * before a label to mark that point.' };
function nmRefresh() {
  nmL = nmLayout();
  nmTL = nmTimeline(nmL);
  const { OW, OH } = dims(nm.out), n = nmL.rows.length;
  $('#nm-fmt').textContent = `${OW} × ${OH} · ${n} row${n === 1 ? '' : 's'} · ${nm.out.fps} fps`;
  $('#nm-uses').textContent = USES[nm.kind];
  const lines = nm.data.split('\n').filter(l => l.trim()).length;
  $('#nm-data-note').textContent = lines > n ? `${lines - n} line${lines - n === 1 ? '' : 's'} without a number ${lines - n === 1 ? 'was' : 'were'} skipped.` : '';
  $('#nm-sample-tag').hidden = nm.data !== NM_SAMPLE;
  toolNm.dataset.kind = nm.kind;
  nmRenderTrack();
  if (nmPlayer.t > nmTL.total) nmPlayer.seek(nmTL.total); else { nmPaint(nmPlayer.t); nmPlayer.sync(); }
  saveSoon('nm', nm);
}
function nmFit() { fitViewer(nmMonitor, nmViewer, nm.out.aspect); nmPaint(nmPlayer.t); }
async function nmEnsureFonts() {
  try { await Promise.all([500, 600, 700, 800].map(w => document.fonts.load(`${w} 40px ${FONTS[nm.font]}`, '0123456789$%Ag'))); } catch (e) { /* fall back silently */ }
}

/* ---------- controls ---------- */
const nmChanged = key => {
  if (key === 'out.aspect') nmFit();
  if (key === 'font') nmEnsureFonts().then(nmRefresh);
  nmRefresh();
};
$('#nm-data').value = nm.data;
$('#nm-data').addEventListener('input', e => { nm.data = e.target.value; nmRefresh(); });
$('#nm-title').value = nm.title;
$('#nm-title').addEventListener('input', e => { nm.title = e.target.value; nmRefresh(); });
$('#nm-source').value = nm.source;
$('#nm-source').addEventListener('input', e => { nm.source = e.target.value; nmRefresh(); });
$('#nm-clear').addEventListener('click', () => { nm.data = ''; nm.title = ''; nm.source = ''; ['#nm-data', '#nm-title', '#nm-source'].forEach(s => { $(s).value = ''; }); nmRefresh(); $('#nm-data').focus(); });
bindSeg('nm-kind', nm, 'kind', nmChanged);
bindCheck('nm-zero', nm, 'zero', nmChanged);
bindCheck('nm-sort', nm, 'sortBars', nmChanged);
bindColor('nm-bg', nm, 'bg', nmChanged);
bindColor('nm-fg', nm, 'fg', nmChanged);
bindColor('nm-muted', nm, 'muted', nmChanged);
bindColor('nm-accent', nm, 'accent', nmChanged);
bindColor('nm-barcolor', nm, 'barColor', nmChanged);
bindSelect('nm-font', nm, 'font', nmChanged);
bindSeg('nm-ease', nm, 'ease', nmChanged);
bindRange('nm-delay', nm, 'delay', fmtSec, nmChanged);
bindRange('nm-build', nm, 'build', fmtSec, nmChanged);
bindRange('nm-stagger', nm, 'stagger', fmtSec, nmChanged);
bindRange('nm-hold', nm, 'hold', fmtSec, nmChanged);

const nmPlayer = makePlayer('nm', { total: () => (nmTL ? nmTL.total : 1), fps: () => nm.out.fps, render: nmPaint, track: $('#nm-track') });

/* ---------- export ---------- */
async function nmRenderOne() {
  nmPlayer.pause();
  await nmEnsureFonts(); nmRefresh();
  if (!nmL.rows.length) { toast('Type at least one row with a number, like  Label | 42'); return 'error'; }
  const L = nmL, TL = nmTL, { q, OW, OH } = dims(nm.out);
  return runRender({
    title: `Rendering ${nm.kind === 'counter' ? 'counter' : nm.kind === 'ring' ? 'ring' : nm.kind + ' chart'}`, OW, OH, fps: nm.out.fps, total: TL.total, format: nm.out.format,
    name: `numbers_${nm.kind}_${slug(nm.title || L.rows[0].label || 'chart') || 'chart'}_${OW}x${OH}`,
    frame: (ctx, t) => { ctx.setTransform(q, 0, 0, q, 0, 0); nmDraw(ctx, L.W, L.H, t, L, TL); },
  });
}
async function nmStill() {
  await nmEnsureFonts(); nmRefresh();
  const { q, OW, OH } = dims(nm.out);
  const c = document.createElement('canvas'); c.width = OW; c.height = OH;
  const ctx = c.getContext('2d'); ctx.setTransform(q, 0, 0, q, 0, 0);
  nmDraw(ctx, nmL.W, nmL.H, nmPlayer.t, nmL, nmTL);
  await saveStill(c, `numbers_frame_${OW}x${OH}.png`);
}
buildOutputPanel('nm', nm.out, nmChanged, {
  render: nmRenderOne,
  renderBoth: () => renderBoth(nm.out, () => { nmFit(); nmRefresh(); }, nmRenderOne),
  still: nmStill,
});
buildPresetPanel('nm', nm, NM_PRESET_KEYS, NM_BUILTINS, () => { resync(nm); resync(nm.out); nmEnsureFonts().then(nmRefresh); nmFit(); nmRefresh(); });
new ResizeObserver(() => { if (!toolNm.hidden) nmFit(); }).observe(nmMonitor);

CS.tools.nm = {
  id: 'nm',
  player: nmPlayer,
  boot() { nmRefresh(); nmPlayer.seek(nmTL.total); nmEnsureFonts().then(nmRefresh); },
  show() { requestAnimationFrame(nmFit); },
  hide() { nmPlayer.pause(); },
  debug: { nm, nmPlayer, get nmL() { return nmL; }, get nmTL() { return nmTL; }, nmParseValue: parseValue, nmFmtValue: fmtValue },
};
})();
