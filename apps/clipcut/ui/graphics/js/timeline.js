/* Clip Studio · Timeline tool
   Classic script (not an ES module) so index.html still works when opened straight from disk.
   Shared helpers live on window.CS; each tool registers itself in CS.tools.

   Events are typed one per line as "Date | Title | Detail" (tabs work too, for rows pasted from a spreadsheet).
   They sit along a line in world coordinates (design pixels): across the frame for wide formats, down it for tall ones.
   The line draws itself, then the camera glides from event to event; each dot pops, its date appears, and its card
   slides in. tlLayout → tlTimeline → tlDraw, shared by preview and export. */
(() => {
'use strict';
const {
  $, clamp, lerp, EASE, easeOutBack, fmtSec, slug, clone, store, toast, roundRect, loadImageFile,
  FRAMES, dims, resync, saveSoon, bindRange, bindColor, bindCheck, bindSeg, bindSelect,
  buildOutputPanel, buildPresetPanel, makePlayer, fitViewer, drawGuides, runRender, renderBoth, saveStill,
} = window.CS;

const FONTS = {
  'Libre Franklin': '"Libre Franklin", Arial, sans-serif',
  'Inter': 'Inter, Arial, sans-serif',
  'Source Serif 4': '"Source Serif 4", Georgia, serif',
  'Oswald': 'Oswald, "Arial Narrow", sans-serif',
};
const TL_SAMPLE = [
  'Jan 2026 | Bill introduced | The Clean Pipes Act is filed with 40 co-sponsors.',
  'Mar 2026 | Committee hearing | Water utilities and health officials testify.',
  'Jun 2026 | House passes it | The vote is 248 to 181.',
  'Sep 2026 | Senate passes it | 61 to 38, after a two-week debate.',
  'Oct 2026 | Signed into law | Replacement work must finish within ten years.',
].join('\n');
const TL_DEFAULT = {
  text: TL_SAMPLE, heading: 'How the Clean Pipes Act became law',
  dir: 'auto', byDate: false, cards: true, dimPast: true, endOverview: true,
  bg: '#101418', lineColor: '#56606B', accent: '#F4C443', cardBg: '#1D232A', fg: '#F5F2EC', muted: '#A7B0BA',
  font: 'Libre Franklin', titleSize: 52,
  intro: 0.8, move: 0.8, hold: 1.5, endHold: 2,
  out: { aspect: '16:9', quality: '1080', fps: 30, format: 'mp4' },
};
const TL_PRESET_KEYS = ['dir', 'byDate', 'cards', 'dimPast', 'endOverview', 'bg', 'lineColor', 'accent', 'cardBg', 'fg', 'muted', 'font', 'titleSize', 'intro', 'move', 'hold', 'endHold', 'out'];
const TL_BUILTINS = {
  'Night desk': { cards: true, dimPast: true, bg: '#101418', lineColor: '#56606B', accent: '#F4C443', cardBg: '#1D232A', fg: '#F5F2EC', muted: '#A7B0BA', font: 'Libre Franklin', titleSize: 52 },
  'Paper': { cards: true, dimPast: true, bg: '#EFEBE3', lineColor: '#B9B2A6', accent: '#C8102E', cardBg: '#FFFFFF', fg: '#161514', muted: '#5E5A54', font: 'Source Serif 4', titleSize: 52 },
  'Breaking red': { cards: false, dimPast: true, bg: '#0B0B0C', lineColor: '#3A3A3D', accent: '#FF3B30', cardBg: '#1A1A1C', fg: '#FFFFFF', muted: '#B5B5BA', font: 'Oswald', titleSize: 54 },
  'Vertical explainer (9:16)': { dir: 'auto', cards: true, dimPast: true, endOverview: false, bg: '#101418', lineColor: '#56606B', accent: '#F4C443', cardBg: '#1D232A', fg: '#F5F2EC', muted: '#A7B0BA', font: 'Inter', titleSize: 50, out: { aspect: '9:16' } },
};

const tl = Object.assign(clone(TL_DEFAULT), store.get('tl', {}));
tl.out = Object.assign({}, TL_DEFAULT.out, tl.out || {});
if (!FRAMES[tl.out.aspect]) tl.out.aspect = '16:9';
if (!FONTS[tl.font]) tl.font = TL_DEFAULT.font;
const photos = [];   // photos[i] for event i; like other media they aren't kept between sessions

const toolTl = $('#tool-tl');
const tlCanvas = $('#tl-canvas'), tlGuides = $('#tl-guides'), tlViewer = $('#tl-viewer'), tlMonitor = $('#tl-monitor');
const mctx = document.createElement('canvas').getContext('2d');
let tlL = null, tlTL = null;

/* ---------- events ---------- */
function parseEvents(text) {
  return text.split('\n').map(l => l.trim()).filter(Boolean).map(line => {
    const parts = line.split(/\s*\|\s*|\t/).map(p => p.trim());
    if (parts.length === 1) return { date: '', title: parts[0], detail: '' };
    return { date: parts[0], title: parts[1] || '', detail: parts.slice(2).join(' · ') };
  }).slice(0, 40);
}
/* A sortable number for a date label, or NaN: full dates, "Mar 2026", or a bare year */
function dateValue(s) {
  if (!s) return NaN;
  const y = s.match(/^\s*(\d{4})\s*$/);
  if (y) return Date.UTC(+y[1], 0, 1);
  const d = Date.parse(s);
  if (!isNaN(d)) return d;
  const my = s.match(/^([A-Za-z]{3,})\.?\s+(\d{4})$/);
  return my ? Date.parse(`1 ${my[1]} ${my[2]}`) : NaN;
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

/* ---------- layout (world coordinates, design pixels) ---------- */
const across = (W, H) => (tl.dir === 'auto' ? W >= H : tl.dir === 'across');
function tlLayout() {
  const { W, H } = dims(tl.out);
  const minSide = Math.min(W, H), horiz = across(W, H);
  const stack = FONTS[tl.font];
  const ts = tl.titleSize * minSide / 1080, ds = ts * 0.62, dateSize = ts * 0.56;
  const pad = ts * 0.6;
  const cardW = horiz ? Math.max(440, Math.min(760, W * 0.36)) : W - minSide * 0.34;
  const events = parseEvents(tl.text).map((ev, i) => {
    const inner = cardW - (tl.cards ? pad * 2 : 0);
    mctx.font = `700 ${ts}px ${stack}`; const titleLines = wrap(ev.title, inner).slice(0, 4);
    mctx.font = `400 ${ds}px ${stack}`; const detailLines = ev.detail ? wrap(ev.detail, inner).slice(0, 5) : [];
    const img = photos[i];
    const imgH = img ? cardW * 0.56 : 0;
    const textH = titleLines.length * ts * 1.15 + (detailLines.length ? ds * 0.5 + detailLines.length * ds * 1.3 : 0);
    const cardH = imgH + textH + (tl.cards ? pad * 2 : 0);
    return { ...ev, i, titleLines, detailLines, img, imgH, cardH, value: dateValue(ev.date) };
  });
  // Positions along the line. Across: even steps, or proportional to the dates. Down: each card's own height.
  const step = cardW + minSide * 0.1;
  let pos = 0;
  const dated = tl.byDate && horiz && events.length > 2 && events.every(e => !isNaN(e.value));
  const gaps = dated ? events.slice(1).map((e, k) => Math.max(0, e.value - events[k].value)) : [];
  const meanGap = gaps.length ? gaps.reduce((a, b) => a + b, 0) / gaps.length || 1 : 1;
  events.forEach((e, k) => {
    if (k) pos += horiz ? (dated ? step * clamp(gaps[k - 1] / meanGap, 0.75, 2.4) : step) : Math.max(events[k - 1].cardH + dateSize * 2.4, minSide * 0.3);
    e.pos = pos;
  });
  // The line runs along y = lineY (across) or x = lineX (down); the camera moves along it
  const lineY = H * 0.4, lineX = minSide * 0.14;
  return { W, H, minSide, horiz, stack, ts, ds, dateSize, pad, cardW, events, lineY, lineX, length: pos };
}
/* Where event i sits on screen at camera offset c (the camera only slides along the line, plus a scale for the overview) */
const anchor = (L, e) => (L.horiz ? { x: e.pos, y: L.lineY } : { x: L.lineX, y: e.pos });

/* ---------- timeline and camera ---------- */
function tlTimeline(L) {
  const n = L.events.length;
  let t = tl.intro;
  const items = L.events.map((e, i) => {
    const t0 = i === 0 ? t : t + tl.move;           // arrive
    const t1 = t0 + 0.5 + tl.hold;                    // leave
    t = t1;
    return { e, t0, t1 };
  });
  const overview = tl.endOverview && n > 1;
  const end = t + (overview ? 1.1 + tl.endHold : tl.endHold * 0.5);
  return { items, overviewAt: overview ? t : null, total: Math.max(1, end) };
}
/* Camera: offset along the line (world units at the focus point) and scale */
function camAt(L, TL, t) {
  const focusFrac = L.horiz ? 0.36 : 0.36;    // the current event sits a bit before the middle, leaving room for the next
  const view = L.horiz ? L.W : L.H;
  const keyOf = i => ({ c: L.events[i].pos, s: 1 });
  if (!TL.items.length) return { c: 0, s: 1, focusFrac };
  let cam = keyOf(0);
  for (let i = 1; i < TL.items.length; i++) {
    const a = TL.items[i - 1].t1, b = TL.items[i].t0;
    if (t >= b) cam = keyOf(i);
    else if (t > a) { cam = { c: lerp(L.events[i - 1].pos, L.events[i].pos, EASE.smooth((t - a) / (b - a))), s: 1 }; break; }
    else break;
  }
  if (TL.overviewAt != null && t > TL.overviewAt) {
    // Zoom out so the whole line fits, centered
    const span = L.length + (L.horiz ? L.cardW : L.events[L.events.length - 1].cardH + L.dateSize * 2);
    const s = clamp(view * 0.86 / Math.max(1, span), 0.2, 1);
    const mid = L.length / 2 + (L.horiz ? 0 : L.events[L.events.length - 1].cardH / 2);
    const e = EASE.smooth(clamp((t - TL.overviewAt) / 1.1, 0, 1));
    // Interpolate in screen terms: where the focus point lands and how big things are
    const sNow = Math.exp(lerp(Math.log(cam.s), Math.log(s), e));
    return { c: lerp(cam.c, mid, e), s: sNow, focusFrac: lerp(focusFrac, 0.5, e) };
  }
  return { ...cam, focusFrac };
}

/* ---------- drawing ---------- */
function drawCard(ctx, L, e, x, y, a, dim) {
  const { stack, ts, ds, pad, cardW } = L;
  ctx.save();
  ctx.globalAlpha = a * (dim ? 0.5 : 1);
  if (tl.cards) {
    ctx.save(); ctx.shadowColor = 'rgba(0,0,0,0.3)'; ctx.shadowBlur = ts * 0.6; ctx.shadowOffsetY = ts * 0.12;
    ctx.fillStyle = tl.cardBg; roundRect(ctx, x, y, cardW, e.cardH, ts * 0.3); ctx.fill(); ctx.restore();
  }
  let cy = y + (tl.cards ? pad : 0);
  const cx = x + (tl.cards ? pad : 0), inner = cardW - (tl.cards ? pad * 2 : 0);
  if (e.img) {
    const iw = e.img.naturalWidth, ih = e.img.naturalHeight, k = Math.max(inner / iw, e.imgH / ih);
    ctx.save(); roundRect(ctx, cx, cy, inner, e.imgH - pad * 0.6, ts * 0.18); ctx.clip();
    ctx.drawImage(e.img, cx + (inner - iw * k) / 2, cy + (e.imgH - pad * 0.6 - ih * k) / 2, iw * k, ih * k);
    ctx.restore();
    cy += e.imgH;
  }
  ctx.textBaseline = 'alphabetic'; ctx.textAlign = 'left';
  ctx.fillStyle = tl.fg; ctx.font = `700 ${ts}px ${stack}`;
  e.titleLines.forEach((ln, k) => ctx.fillText(ln, cx, cy + ts * 0.9 + k * ts * 1.15));
  cy += e.titleLines.length * ts * 1.15;
  if (e.detailLines.length) {
    ctx.fillStyle = tl.muted; ctx.font = `400 ${ds}px ${stack}`;
    e.detailLines.forEach((ln, k) => ctx.fillText(ln, cx, cy + ds * 0.5 + ds * 0.95 + k * ds * 1.3));
  }
  ctx.restore();
}
function tlDraw(ctx, W, H, t, L, TL) {
  ctx.save();
  ctx.fillStyle = tl.bg; ctx.fillRect(0, 0, W, H);
  const cam = camAt(L, TL, t);
  const fx = L.horiz ? W * cam.focusFrac : W / 2, fy = L.horiz ? H / 2 : H * cam.focusFrac;
  // World → screen: the camera point (c along the line) lands at (fx, fy) at scale s
  ctx.save();
  ctx.translate(fx, fy); ctx.scale(cam.s, cam.s);
  if (L.horiz) ctx.translate(-cam.c, -H / 2); else ctx.translate(-W / 2, -cam.c);
  const lw = Math.max(3, L.minSide * 0.005);
  // Line: grows from before the first event to the event being reached
  const first = L.events[0], lastIdx = TL.items.findIndex(it => it.t0 > t);
  const reached = lastIdx < 0 ? TL.items.length - 1 : lastIdx - 1;
  let head;
  if (!TL.items.length) head = 0;
  else {
    const intro = EASE.smooth(clamp(t / Math.max(0.1, tl.intro), 0, 1));
    head = lerp(-L.minSide * 0.5, first.pos, intro);
    for (let i = 1; i < TL.items.length; i++) {
      const a = TL.items[i - 1].t1, b = TL.items[i].t0;
      if (t >= b) head = L.events[i].pos;
      else if (t > a) { head = lerp(L.events[i - 1].pos, L.events[i].pos, EASE.smooth((t - a) / (b - a))); break; }
    }
    if (TL.overviewAt != null && t > TL.overviewAt) head = lerp(head, L.length + L.minSide * 0.3, EASE.smooth(clamp((t - TL.overviewAt) / 1.1, 0, 1)));
  }
  // The rest of the line, faint and dashed, shows where the story is going
  const far = L.length + L.minSide * 0.3;
  if (head < far) {
    ctx.save(); ctx.strokeStyle = tl.lineColor; ctx.globalAlpha = 0.35; ctx.lineWidth = lw * 0.7; ctx.setLineDash([lw * 2.5, lw * 3]);
    ctx.beginPath();
    if (L.horiz) { ctx.moveTo(head, L.lineY); ctx.lineTo(far, L.lineY); } else { ctx.moveTo(L.lineX, head); ctx.lineTo(L.lineX, far); }
    ctx.stroke(); ctx.restore();
  }
  ctx.strokeStyle = tl.lineColor; ctx.lineWidth = lw; ctx.lineCap = 'round';
  ctx.beginPath();
  if (L.horiz) { ctx.moveTo(-L.minSide * 0.5, L.lineY); ctx.lineTo(head, L.lineY); }
  else { ctx.moveTo(L.lineX, -L.minSide * 0.5); ctx.lineTo(L.lineX, head); }
  ctx.stroke();
  // Events
  TL.items.forEach((it, i) => {
    const age = t - it.t0;
    if (age < 0) return;
    const e = it.e, A = anchor(L, e);
    const dim = tl.dimPast && i < reached && (TL.overviewAt == null || t < TL.overviewAt);
    const pop = easeOutBack(clamp(age / 0.35, 0, 1));
    const r = L.minSide * 0.016;
    ctx.save();
    ctx.globalAlpha = dim ? 0.6 : 1;
    ctx.fillStyle = tl.bg; ctx.beginPath(); ctx.arc(A.x, A.y, (r + lw) * pop, 0, Math.PI * 2); ctx.fill();
    ctx.fillStyle = tl.accent; ctx.beginPath(); ctx.arc(A.x, A.y, r * pop, 0, Math.PI * 2); ctx.fill();
    ctx.restore();
    const a = EASE.smooth(clamp((age - 0.1) / 0.45, 0, 1));
    if (a <= 0) return;
    const slide = (1 - a) * L.ts * 0.8;
    ctx.save();
    ctx.globalAlpha = a * (dim ? 0.5 : 1);
    ctx.fillStyle = tl.accent; ctx.font = `700 ${L.dateSize}px ${L.stack}`; ctx.textBaseline = 'alphabetic';
    if ('letterSpacing' in ctx) ctx.letterSpacing = `${(L.dateSize * 0.06).toFixed(1)}px`;
    const date = e.date.toUpperCase();
    if (L.horiz) { ctx.textAlign = 'center'; ctx.fillText(date, A.x, A.y - r * 2.2 - slide * 0.5); }
    else { ctx.textAlign = 'left'; ctx.fillText(date, A.x + r * 2.4, A.y + L.dateSize * 0.36); }
    ctx.restore();
    if (L.horiz) drawCard(ctx, L, e, A.x - L.cardW / 2, A.y + r * 2.6 + slide, a, dim);
    else drawCard(ctx, L, e, A.x + r * 2.4, A.y + L.dateSize * 1.2 + slide, a, dim);
  });
  ctx.restore();
  // Heading, pinned to the frame
  const head2 = (tl.heading || '').trim();
  if (head2) {
    const hs = L.minSide * 0.034, m = L.minSide * 0.06;
    const a = EASE.smooth(clamp(t / 0.5, 0, 1));
    // A band of background color behind the heading, so events scrolling up pass under it
    const lines = (mctx.font = `700 ${hs}px ${L.stack}`, wrap(head2, W - 2 * m).slice(0, 2));
    const bandH = m + hs * 1.2 * lines.length + hs * 0.9;
    const fadeTo = bandH + hs * 3;
    const g = ctx.createLinearGradient(0, 0, 0, fadeTo);
    g.addColorStop(0, tl.bg); g.addColorStop((bandH + hs * 0.6) / fadeTo, tl.bg); g.addColorStop(1, tl.bg + '00');
    ctx.fillStyle = g; ctx.fillRect(0, 0, W, fadeTo);
    ctx.save(); ctx.globalAlpha = a;
    ctx.font = `700 ${hs}px ${L.stack}`; ctx.fillStyle = tl.fg; ctx.textBaseline = 'alphabetic'; ctx.textAlign = 'left';
    lines.forEach((ln, k) => ctx.fillText(ln, m, m + hs + k * hs * 1.2));
    ctx.fillStyle = tl.accent; ctx.fillRect(m, m - hs * 0.55, hs * 1.6, Math.max(3, hs * 0.12));
    ctx.restore();
  }
  ctx.restore();
}

/* ---------- preview ---------- */
function tlPaint(t) {
  if (!tlL) return;
  const { W, H } = tlL;
  if (tlCanvas.width !== W || tlCanvas.height !== H) { tlCanvas.width = W; tlCanvas.height = H; }
  const ctx = tlCanvas.getContext('2d');
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  tlDraw(ctx, W, H, t, tlL, tlTL);
  const g = tlGuides.getContext('2d');
  const dpr = window.devicePixelRatio || 1;
  const gw = Math.round(tlViewer.clientWidth * dpr), gh = Math.round(tlViewer.clientHeight * dpr);
  if (tlGuides.width !== gw || tlGuides.height !== gh) { tlGuides.width = gw; tlGuides.height = gh; }
  g.clearRect(0, 0, gw, gh);
  if (tlPlayer.guides) drawGuides(g, gw, gh, tl.out.aspect, gw / W * 2);
}
function tlRenderTrack() {
  const tr = $('#tl-track'), T = tlTL.total;
  const blk = (from, len, c, label, dim) => `<div class="blk${dim ? ' dim' : ''}" style="left:${from / T * 100}%;width:${Math.max(0.3, len / T * 100)}%;--c:${c}">${label ? `<span>${label}</span>` : ''}</div>`;
  let html = blk(0, tl.intro, '#8a857c', 'In', true);
  tlTL.items.forEach((it, i) => { html += blk(it.t0, it.t1 - it.t0, tl.accent, String(i + 1)); });
  if (tlTL.overviewAt != null) html += blk(tlTL.overviewAt, tlTL.total - tlTL.overviewAt, '#8a857c', 'All', true);
  tr.innerHTML = html + '<div class="head"></div>';
}
function tlRefresh() {
  tlL = tlLayout();
  tlTL = tlTimeline(tlL);
  const { OW, OH } = dims(tl.out), n = tlL.events.length;
  $('#tl-fmt').textContent = `${OW} × ${OH} · ${n} event${n === 1 ? '' : 's'} · ${tlL.horiz ? 'across' : 'down'} · ${tl.out.fps} fps`;
  $('#tl-sample-tag').hidden = tl.text !== TL_SAMPLE;
  const allDated = tlL.events.length > 2 && tlL.events.every(e => !isNaN(e.value));
  $('#tl-bydate-note').textContent = tl.byDate && tlL.horiz && !allDated ? 'Spacing by date needs a date this tool can read on every line (e.g. 2026-03-14, Mar 2026, or 2026).' : '';
  $('#tl-outnote').textContent = '';
  tlPhotosUI();
  tlRenderTrack();
  if (tlPlayer.t > tlTL.total) tlPlayer.seek(tlTL.total); else { tlPaint(tlPlayer.t); tlPlayer.sync(); }
  saveSoon('tl', tl);
}
function tlFit() { fitViewer(tlMonitor, tlViewer, tl.out.aspect); tlPaint(tlPlayer.t); }
async function tlEnsureFonts() {
  try { await Promise.all([400, 700].map(w => document.fonts.load(`${w} 40px ${FONTS[tl.font]}`, 'Ag'))); } catch (e) { /* fall back silently */ }
}

/* Photos: one optional photo per event, listed under the text box */
function tlPhotosUI() {
  const host = $('#tl-photos');
  const evs = tlL.events;
  if (host.dataset.sig === evs.map(e => e.title).join('|') + photos.map(p => !!p).join('')) return;
  host.dataset.sig = evs.map(e => e.title).join('|') + photos.map(p => !!p).join('');
  host.innerHTML = '';
  evs.forEach((e, i) => {
    const li = document.createElement('li');
    li.innerHTML = `<span class="n">${i + 1}</span><span class="t"></span><button class="btn small" type="button"></button><input type="file" accept="image/*" hidden>`;
    li.querySelector('.t').textContent = e.title || e.date || `Event ${i + 1}`;
    const btn = li.querySelector('button'), file = li.querySelector('input');
    btn.textContent = photos[i] ? 'Remove photo' : 'Add photo';
    btn.setAttribute('aria-label', `${photos[i] ? 'Remove the photo from' : 'Add a photo to'} event ${i + 1}`);
    btn.addEventListener('click', () => { if (photos[i]) { photos[i] = null; tlRefresh(); } else file.click(); });
    file.addEventListener('change', async ev => {
      const f = ev.target.files[0]; ev.target.value = '';
      if (!f) return;
      try { photos[i] = await loadImageFile(f); tlRefresh(); } catch (err) { toast(err.message); }
    });
    host.appendChild(li);
  });
}

/* ---------- controls ---------- */
const tlChanged = key => {
  if (key === 'out.aspect') tlFit();
  if (key === 'font') tlEnsureFonts().then(tlRefresh);
  tlRefresh();
};
$('#tl-text').value = tl.text;
$('#tl-text').addEventListener('input', e => { tl.text = e.target.value; tlRefresh(); });
$('#tl-sample-clear').addEventListener('click', () => { tl.text = ''; $('#tl-text').value = ''; photos.length = 0; tlRefresh(); $('#tl-text').focus(); });
const bindHeading = () => {
  const el = $('#tl-heading'); el.value = tl.heading;
  el.addEventListener('input', () => { tl.heading = el.value; tlRefresh(); });
};
bindHeading();
bindSeg('tl-dir', tl, 'dir', tlChanged);
bindCheck('tl-bydate', tl, 'byDate', tlChanged);
bindCheck('tl-cards', tl, 'cards', tlChanged);
bindCheck('tl-dimpast', tl, 'dimPast', tlChanged);
bindCheck('tl-endoverview', tl, 'endOverview', tlChanged);
bindColor('tl-bg', tl, 'bg', tlChanged);
bindColor('tl-line', tl, 'lineColor', tlChanged);
bindColor('tl-accent', tl, 'accent', tlChanged);
bindColor('tl-cardbg', tl, 'cardBg', tlChanged);
bindColor('tl-fg', tl, 'fg', tlChanged);
bindColor('tl-muted', tl, 'muted', tlChanged);
bindSelect('tl-font', tl, 'font', tlChanged);
bindRange('tl-size', tl, 'titleSize', v => v + 'px', tlChanged);
bindRange('tl-intro', tl, 'intro', fmtSec, tlChanged);
bindRange('tl-move', tl, 'move', fmtSec, tlChanged);
bindRange('tl-hold', tl, 'hold', fmtSec, tlChanged);
bindRange('tl-endhold', tl, 'endHold', fmtSec, tlChanged);

const tlPlayer = makePlayer('tl', { total: () => (tlTL ? tlTL.total : 1), fps: () => tl.out.fps, render: tlPaint, track: $('#tl-track') });

/* ---------- export ---------- */
async function tlRenderOne() {
  tlPlayer.pause();
  await tlEnsureFonts(); tlRefresh();
  if (!tlL.events.length) { toast('Type at least one event first.'); return 'error'; }
  const L = tlL, TL = tlTL, { q, OW, OH } = dims(tl.out);
  return runRender({
    title: `Rendering ${L.events.length}-event timeline`, OW, OH, fps: tl.out.fps, total: TL.total, format: tl.out.format,
    name: `timeline_${slug(tl.heading || L.events[0].title || 'events') || 'events'}_${OW}x${OH}`,
    frame: (ctx, t) => { ctx.setTransform(q, 0, 0, q, 0, 0); tlDraw(ctx, L.W, L.H, t, L, TL); },
  });
}
async function tlStill() {
  await tlEnsureFonts(); tlRefresh();
  const { q, OW, OH } = dims(tl.out);
  const c = document.createElement('canvas'); c.width = OW; c.height = OH;
  const ctx = c.getContext('2d'); ctx.setTransform(q, 0, 0, q, 0, 0);
  tlDraw(ctx, tlL.W, tlL.H, tlPlayer.t, tlL, tlTL);
  await saveStill(c, `timeline_frame_${OW}x${OH}.png`);
}
buildOutputPanel('tl', tl.out, tlChanged, {
  render: tlRenderOne,
  renderBoth: () => renderBoth(tl.out, () => { tlFit(); tlRefresh(); }, tlRenderOne),
  still: tlStill,
});
buildPresetPanel('tl', tl, TL_PRESET_KEYS, TL_BUILTINS, () => { resync(tl); resync(tl.out); tlEnsureFonts().then(tlRefresh); tlFit(); tlRefresh(); });
new ResizeObserver(() => { if (!toolTl.hidden) tlFit(); }).observe(tlMonitor);

CS.tools.tl = {
  id: 'tl',
  player: tlPlayer,
  boot() {
    tlRefresh();
    tlPlayer.seek(tlTL.items.length > 2 ? tlTL.items[2].t0 + 0.8 : 0);
    tlEnsureFonts().then(tlRefresh);
  },
  show() { requestAnimationFrame(tlFit); },
  hide() { tlPlayer.pause(); },
  debug: { tl, tlPlayer, get tlL() { return tlL; }, get tlTL() { return tlTL; } },
};
})();
