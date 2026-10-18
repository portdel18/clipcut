/* Clip Studio · Audiogram tool (audio clips with a waveform, speaker card, and captions)
   Classic script (not an ES module) so index.html still works when opened straight from disk.
   Shared helpers live on window.CS; each tool registers itself in CS.tools.

   The audio (or a video's soundtrack) is decoded once at 48 kHz. A loudness envelope at 60 values per second drives
   the waveform, so the preview and the export draw the same thing for the same moment. The preview plays sound from an
   <audio> element and resyncs drift; the export hands the trimmed AudioBuffer to the shared MP4 writer.

   Captions come from the text box: an .srt or .vtt (anything with "-->") keeps its own times; plain text is cut into
   short captions and spread over the parts of the clip where someone is talking (silences are skipped).
   auCues → auDraw, shared by preview and export. */
(() => {
'use strict';
const {
  $, clamp, lerp, EASE, easeOutBack, fmtSec, slug, clone, store, toast, roundRect, loadImageFile,
  FRAMES, dims, resync, saveSoon, bindRange, bindColor, bindCheck, bindSeg, bindSelect, bindText,
  buildOutputPanel, buildPresetPanel, makePlayer, fitViewer, drawGuides, runRender, renderBoth, saveStill,
} = window.CS;

const FONTS = {
  'Inter': 'Inter, Arial, sans-serif',
  'Libre Franklin': '"Libre Franklin", Arial, sans-serif',
  'Montserrat': 'Montserrat, Arial, sans-serif',
  'Oswald': 'Oswald, "Arial Narrow", sans-serif',
};
const ENV_RATE = 60, MAX_LEN = 600, MAX_BYTES = 300e6;
const AU_SAMPLE_CAPS = 'This is sample audio with typed captions. Choose an audio or video file, then paste what is said, or import an .srt or .vtt file from your editor. Plain text is spread over the parts where someone is talking.';
const AU_DEFAULT = {
  name: 'Sample speaker', role: 'Sample audio', source: '',
  captions: AU_SAMPLE_CAPS,
  style: 'bars', capMode: 'word', capWords: 7, capSize: 64, capPos: 'bottom', capBox: true, upper: false, showPhoto: true, progress: true,
  bg: '#0E1116', fg: '#F5F2EC', muted: '#9AA4AF', wave: '#F4C443', hiWord: '#F4C443',
  font: 'Inter', length: 0,
  out: { aspect: '16:9', quality: '1080', fps: 30, format: 'mp4' },
};
const AU_PRESET_KEYS = ['style', 'capMode', 'capWords', 'capSize', 'capPos', 'capBox', 'upper', 'showPhoto', 'progress', 'bg', 'fg', 'muted', 'wave', 'hiWord', 'font', 'out'];
const AU_BUILTINS = {
  'Night desk': { style: 'bars', capMode: 'word', capBox: true, upper: false, bg: '#0E1116', fg: '#F5F2EC', muted: '#9AA4AF', wave: '#F4C443', hiWord: '#F4C443', font: 'Inter' },
  'Breaking audio': { style: 'line', capMode: 'word', capBox: false, upper: true, bg: '#0B0B0C', fg: '#FFFFFF', muted: '#B5B5BA', wave: '#FF3B30', hiWord: '#FF3B30', font: 'Oswald' },
  'Podcast clip (9:16)': { style: 'pulse', capMode: 'pop', capWords: 4, capSize: 78, capPos: 'middle', capBox: false, upper: false, bg: '#14101C', fg: '#FFFFFF', muted: '#B9AFCB', wave: '#BF5AF2', hiWord: '#FFD60A', font: 'Montserrat', out: { aspect: '9:16' } },
  'Plain captions': { style: 'bars', capMode: 'line', capBox: true, upper: false, bg: '#000000', fg: '#FFFFFF', muted: '#A1A1A6', wave: '#FFFFFF', hiWord: '#FFFFFF', font: 'Libre Franklin' },
};

const au = Object.assign(clone(AU_DEFAULT), store.get('au', {}));
au.out = Object.assign({}, AU_DEFAULT.out, au.out || {});
if (!FRAMES[au.out.aspect]) au.out.aspect = '16:9';
if (!FONTS[au.font]) au.font = AU_DEFAULT.font;

let media = null;          // { name, buf, env, voiced, url, el }; never stored
let trimIn = 0;            // belongs to one file, so not stored either
let photo = null;
let auTL = null;

const toolAu = $('#tool-au');
const auCanvas = $('#au-canvas'), auGuides = $('#au-guides'), auViewer = $('#au-viewer'), auMonitor = $('#au-monitor');
const mctx = document.createElement('canvas').getContext('2d');

/* ---------- audio analysis ---------- */
function envelope(buf) {
  const n = Math.ceil(buf.duration * ENV_RATE), step = buf.sampleRate / ENV_RATE;
  const chans = Array.from({ length: buf.numberOfChannels }, (_, c) => buf.getChannelData(c));
  const env = new Float32Array(n);
  for (let k = 0; k < n; k++) {
    const a = Math.floor(k * step), b = Math.min(buf.length, Math.floor((k + 1) * step));
    let sum = 0;
    for (const ch of chans) for (let i = a; i < b; i += 2) sum += ch[i] * ch[i];
    env[k] = Math.sqrt(sum / Math.max(1, ((b - a) / 2) * chans.length));
  }
  // Normalize to the loud end of the clip (95th percentile) and soften, so quiet speech still moves the bars
  const sorted = Float32Array.from(env).sort();
  const ref = sorted[Math.floor(sorted.length * 0.95)] || 1e-6;
  for (let k = 0; k < n; k++) env[k] = Math.min(1, Math.sqrt(env[k] / ref));
  return env;
}
/* Talking or not, per envelope step: above a threshold, with pauses shorter than 0.25 s bridged */
function voicedOf(env) {
  const v = new Uint8Array(env.length), gap = Math.round(0.25 * ENV_RATE);
  for (let k = 0; k < env.length; k++) v[k] = env[k] > 0.3 ? 1 : 0;
  let last = -1;
  for (let k = 0; k < v.length; k++) {
    if (!v[k]) continue;
    if (last >= 0 && k - last > 1 && k - last <= gap) for (let j = last + 1; j < k; j++) v[j] = 1;
    last = k;
  }
  return v;
}
/* A made-up envelope for the sample, so the first view moves: syllables in two phrases with a pause */
const SAMPLE_LEN = 12;
const sampleEnv = (() => {
  const env = new Float32Array(SAMPLE_LEN * ENV_RATE);
  for (let k = 0; k < env.length; k++) {
    const t = k / ENV_RATE, on = (t > 0.4 && t < 5.2) || (t > 6 && t < 11.4);
    env[k] = on ? clamp(0.45 + 0.4 * Math.sin(t * 13.1) * Math.sin(t * 3.7) + 0.15 * Math.sin(t * 29), 0.05, 1) : 0.03;
  }
  return env;
})();
const clipLen = () => {
  if (!media) return SAMPLE_LEN;
  const rest = media.buf.duration - trimIn;
  return clamp(au.length > 0 ? Math.min(au.length, rest) : rest, 0.5, MAX_LEN);
};
const envData = () => (media ? media.env : sampleEnv);
/** Loudness 0..1 at clip time t */
function envAt(t) {
  const env = envData(), x = ((media ? trimIn : 0) + t) * ENV_RATE;
  if (x < 0 || x >= env.length - 1) return 0;
  const k = Math.floor(x), f = x - k;
  return env[k] * (1 - f) + env[k + 1] * f;
}

/* ---------- captions ---------- */
const toSec = s => { const p = s.trim().replace(',', '.').split(':').map(Number); return p.length === 3 ? p[0] * 3600 + p[1] * 60 + p[2] : p.length === 2 ? p[0] * 60 + p[1] : p[0]; };
/** .srt or .vtt → [{ start, end, text }] in file time */
function parseTimed(text) {
  const cues = [];
  for (const block of text.replace(/\r/g, '').split(/\n\s*\n/)) {
    const lines = block.split('\n');
    const i = lines.findIndex(l => l.includes('-->'));
    if (i < 0) continue;
    const [a, b] = lines[i].split('-->');
    const start = toSec(a), end = toSec(b.trim().split(/\s+/)[0]);
    const body = lines.slice(i + 1).join(' ').replace(/<[^>]+>/g, '').replace(/\{[^}]+\}/g, '').replace(/\s+/g, ' ').trim();
    if (body && isFinite(start) && isFinite(end) && end > start) cues.push({ start, end, text: body });
  }
  return cues;
}
/** Plain text → short captions of at most n words, ending early at sentence ends */
function chunkWords(words, n) {
  const out = []; let cur = [];
  for (const w of words) {
    cur.push(w);
    if (cur.length >= n || (/[.?!]["”’)]?$/.test(w) && cur.length >= 3)) { out.push(cur); cur = []; }
  }
  if (cur.length) out.push(cur);
  return out;
}
/** Times for words spread over [t0, t1] by character count */
function timeWords(words, t0, t1) {
  const lens = words.map(w => w.length + 1), total = lens.reduce((a, b) => a + b, 0);
  let acc = 0;
  return words.map((w, k) => { const a = t0 + (t1 - t0) * acc / total; acc += lens[k]; return { text: w, t0: a, t1: t0 + (t1 - t0) * acc / total }; });
}
function auCues() {
  const len = clipLen(), text = au.captions.trim(), n = Math.max(2, Math.round(au.capWords));
  if (!text) return { cues: [], timed: false };
  if (text.includes('-->')) {
    const cues = [];
    for (const c of parseTimed(text)) {
      const s = c.start - (media ? trimIn : 0), e = c.end - (media ? trimIn : 0);
      if (e <= 0 || s >= len) continue;
      // Long cues are split into shorter captions, each given its share of the cue's time
      const words = c.text.split(/\s+/), parts = chunkWords(words, n);
      const chars = parts.map(p => p.join(' ').length), sum = chars.reduce((a, b) => a + b, 0);
      let at = s;
      parts.forEach((p, k) => { const d = (e - s) * chars[k] / sum; cues.push({ t0: at, t1: at + d, words: timeWords(p, at, at + d) }); at += d; });
    }
    return { cues, timed: true };
  }
  // Plain text: spread over talking time. V[k] = seconds of talking before step k of the clip.
  const steps = Math.ceil(len * ENV_RATE), V = new Float32Array(steps + 1);
  const voiced = media ? media.voiced : voicedOf(sampleEnv), off = media ? Math.round(trimIn * ENV_RATE) : 0;
  for (let k = 0; k < steps; k++) V[k + 1] = V[k] + (voiced[off + k] ? 1 / ENV_RATE : 0);
  const talk = V[steps];
  // Clip time for a point on the talking-time clock. A start is placed where the clock passes it (so it lands on
  // the first talking moment after a pause); an end where the clock reaches it (so it ends as the talking stops).
  const timeOf = (v, end) => {
    let lo = 0, hi = steps;
    while (lo < hi) { const mid = (lo + hi) >> 1; if (end ? V[mid] < v : V[mid + 1] <= v) lo = mid + 1; else hi = mid; }
    return Math.min(lo, steps) / ENV_RATE;
  };
  const scale = talk < 0.5 ? len : talk;
  const parts = chunkWords(text.split(/\s+/), n);
  const chars = parts.map(p => p.join(' ').length + 1), sum = chars.reduce((a, b) => a + b, 0);
  let acc = 0;
  const cues = parts.map((p, k) => {
    const v0 = scale * acc / sum; acc += chars[k];
    const v1 = scale * acc / sum;
    const t0 = talk < 0.5 ? v0 : timeOf(v0, false), t1 = talk < 0.5 ? v1 : timeOf(v1, true);
    // Words inside a caption also follow the talking clock, so they don't light up during a pause
    const words = timeWords(p, v0, v1).map(w => ({ text: w.text, t0: talk < 0.5 ? w.t0 : timeOf(w.t0, false), t1: talk < 0.5 ? w.t1 : timeOf(w.t1, true) }));
    return { t0, t1: Math.max(t1, t0 + 0.3), words };
  });
  return { cues, timed: false };
}
/** The caption on screen at t. A caption stays up through a short pause after it, until the next one starts. */
function cueAt(TL, t) {
  const cs = TL.cues;
  for (let k = 0; k < cs.length; k++) {
    const next = cs[k + 1];
    const until = Math.min(next ? next.t0 : Infinity, cs[k].t1 + 0.6);
    if (t >= cs[k].t0 && t < until) return cs[k];
  }
  return null;
}

/* ---------- drawing ---------- */
function wrapWords(words, maxW) {
  const lines = []; let cur = [], w = 0;
  const space = mctx.measureText(' ').width;
  for (const wd of words) {
    const ww = mctx.measureText(wd.text).width;
    if (cur.length && w + space + ww > maxW) { lines.push({ words: cur, w }); cur = []; w = 0; }
    wd.w = ww; cur.push(wd); w += (cur.length > 1 ? space : 0) + ww;
  }
  if (cur.length) lines.push({ words: cur, w });
  return lines;
}
function layoutFor(W, H) {
  const minSide = Math.min(W, H), wide = W > H * 1.1;
  const photoOn = au.showPhoto;
  if (wide) {
    return {
      wide, minSide,
      photo: photoOn ? { cx: W * 0.2, cy: H * 0.38, r: minSide * 0.17 } : null,
      name: { x: photoOn ? W * 0.36 : W / 2, y: H * 0.2, align: photoOn ? 'left' : 'center' },
      wave: { x: photoOn ? W * 0.36 : W * 0.1, w: photoOn ? W * 0.56 : W * 0.8, cy: H * 0.43, h: H * 0.2 },
      cap: { cx: W / 2, cy: au.capPos === 'middle' ? H * 0.62 : H * 0.76, w: W * 0.84 },
      prog: { x: W * 0.08, w: W * 0.84, y: H * 0.93 },
    };
  }
  const r = minSide * 0.19;
  return {
    wide, minSide,
    photo: photoOn ? { cx: W / 2, cy: H * 0.2, r } : null,
    name: { x: W / 2, y: photoOn ? H * 0.2 + r * 1.5 : H * 0.18, align: 'center' },   // below the outer ring
    wave: { x: W * 0.08, w: W * 0.84, cy: photoOn ? H * 0.47 : H * 0.36, h: H * 0.09 },
    cap: { cx: W / 2, cy: au.capPos === 'middle' ? H * 0.6 : H * 0.68, w: W * 0.86 },
    prog: { x: W * 0.08, w: W * 0.84, y: H * 0.9 },
  };
}
function drawWave(ctx, G, t) {
  const { wave } = G, color = au.wave;
  if (au.style === 'pulse') return;   // the pulse lives around the photo
  ctx.save();
  ctx.fillStyle = color; ctx.strokeStyle = color;
  if (au.style === 'bars') {
    // Bars ripple out from the middle: bar i shows the loudness a moment ago, further out = further back
    const N = G.wide ? 48 : 32, gap = wave.w / N, bw = gap * 0.56;
    for (let i = 0; i < N; i++) {
      const d = Math.abs(i - (N - 1) / 2);
      const v = envAt(t - d * 0.035) * (1 - 0.35 * d / N);
      const h = Math.max(bw, v * wave.h);
      roundRect(ctx, wave.x + i * gap + (gap - bw) / 2, wave.cy - h / 2, bw, h, bw / 2); ctx.fill();
    }
  } else {
    // Line: the last 2.5 s scroll right to left, mirrored around the center line
    const span = 2.5, n = 160;
    ctx.beginPath();
    for (let k = 0; k <= n; k++) { const x = wave.x + wave.w * k / n; const y = wave.cy - envAt(t - span + span * k / n) * wave.h / 2; k ? ctx.lineTo(x, y) : ctx.moveTo(x, y); }
    for (let k = n; k >= 0; k--) { const x = wave.x + wave.w * k / n; ctx.lineTo(x, wave.cy + envAt(t - span + span * k / n) * wave.h / 2); }
    ctx.closePath(); ctx.globalAlpha = 0.9; ctx.fill();
  }
  ctx.restore();
}
function drawPhoto(ctx, G, t, stack) {
  const P = G.photo; if (!P) return;
  const v = envAt(t);
  // Rings that swell with the voice (the whole effect in the Pulse style, a subtle halo otherwise)
  const rings = au.style === 'pulse' ? 3 : 1;
  ctx.save();
  for (let k = rings; k >= 1; k--) {
    ctx.globalAlpha = (au.style === 'pulse' ? 0.22 : 0.3) * (1 - (k - 1) / (rings + 1));
    ctx.fillStyle = au.wave;
    ctx.beginPath(); ctx.arc(P.cx, P.cy, P.r * (1 + (0.04 + v * (au.style === 'pulse' ? 0.1 : 0.08)) * k), 0, Math.PI * 2); ctx.fill();   // at most 1.42 × r
  }
  ctx.globalAlpha = 1;
  ctx.save(); ctx.beginPath(); ctx.arc(P.cx, P.cy, P.r, 0, Math.PI * 2); ctx.clip();
  if (photo) {
    const iw = photo.naturalWidth, ih = photo.naturalHeight, k = Math.max(2 * P.r / iw, 2 * P.r / ih);
    ctx.drawImage(photo, P.cx - iw * k / 2, P.cy - ih * k / 2, iw * k, ih * k);
  } else {
    ctx.fillStyle = '#2A3038'; ctx.fillRect(P.cx - P.r, P.cy - P.r, P.r * 2, P.r * 2);
    const initials = (au.name || '').split(/\s+/).filter(Boolean).slice(0, 2).map(w => w[0].toUpperCase()).join('') || '♪';
    ctx.fillStyle = au.muted; ctx.font = `700 ${P.r * 0.7}px ${stack}`; ctx.textAlign = 'center'; ctx.textBaseline = 'middle';
    ctx.fillText(initials, P.cx, P.cy + P.r * 0.04);
  }
  ctx.restore();
  ctx.restore();
}
function drawCaption(ctx, G, TL, t, stack) {
  const cue = cueAt(TL, t);
  if (!cue) return;
  const C = G.cap;
  let size = au.capSize * G.minSide / 1080;
  const words = cue.words.map(w => ({ ...w, text: au.upper ? w.text.toUpperCase() : w.text }));
  let lines;
  for (let i = 0; i < 12; i++) {
    mctx.font = `800 ${size}px ${stack}`;
    lines = wrapWords(words, C.w - size);
    if (lines.length <= 2) break;
    size *= 0.9;
  }
  const lh = size * 1.22, h = lines.length * lh;
  const inT = clamp((t - cue.t0) / 0.15, 0, 1);
  ctx.save();
  ctx.font = `800 ${size}px ${stack}`; ctx.textBaseline = 'middle'; ctx.textAlign = 'left';
  const space = ctx.measureText(' ').width;
  if (au.capBox) {
    const bw = Math.max(...lines.map(l => l.w)) + size * 1.1, bh = h + size * 0.6;
    ctx.globalAlpha = 0.72 * inT; ctx.fillStyle = '#000000';
    roundRect(ctx, C.cx - bw / 2, C.cy - bh / 2, bw, bh, size * 0.3); ctx.fill();
  }
  lines.forEach((ln, li) => {
    let x = C.cx - ln.w / 2;
    const y = C.cy - h / 2 + lh * (li + 0.5);
    for (const w of ln.words) {
      const said = t >= w.t0, now = said && t < w.t1;
      let a = 1, col = au.fg, dy = 0;
      if (au.capMode === 'word') { a = said ? 1 : 0.42; if (now) col = au.hiWord; }
      else if (au.capMode === 'pop') { const e = clamp((t - w.t0) / 0.12, 0, 1); a = e; dy = (1 - EASE.snappy(e)) * size * 0.25; if (now) col = au.hiWord; }
      ctx.globalAlpha = a * inT; ctx.fillStyle = col;
      if (!au.capBox) { ctx.shadowColor = 'rgba(0,0,0,0.6)'; ctx.shadowBlur = size * 0.18; }
      ctx.fillText(w.text, x, y + dy);
      x += w.w + space;
    }
  });
  ctx.restore();
}
const fmtClock = s => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, '0')}`;
function auDraw(ctx, W, H, t, TL) {
  const G = layoutFor(W, H), stack = FONTS[au.font];
  ctx.save();
  ctx.fillStyle = au.bg; ctx.fillRect(0, 0, W, H);
  drawPhoto(ctx, G, t, stack);
  // Speaker card
  const ns = G.minSide * 0.05, rs = G.minSide * 0.03;
  ctx.textAlign = G.name.align; ctx.textBaseline = 'alphabetic';
  if (au.name.trim()) { ctx.fillStyle = au.fg; ctx.font = `800 ${ns}px ${stack}`; ctx.fillText(au.name.trim(), G.name.x, G.name.y + ns); }
  if (au.role.trim()) { ctx.fillStyle = au.muted; ctx.font = `500 ${rs}px ${stack}`; ctx.fillText(au.role.trim(), G.name.x, G.name.y + ns + rs * 1.6); }
  drawWave(ctx, G, t);
  drawCaption(ctx, G, TL, t, stack);
  if (au.progress) {
    const P = G.prog, lw = Math.max(3, G.minSide * 0.005), f = clamp(t / TL.total, 0, 1);
    ctx.globalAlpha = 0.3; ctx.fillStyle = au.muted; roundRect(ctx, P.x, P.y, P.w, lw, lw / 2); ctx.fill();
    ctx.globalAlpha = 1; ctx.fillStyle = au.wave; roundRect(ctx, P.x, P.y, Math.max(lw, P.w * f), lw, lw / 2); ctx.fill();
    ctx.fillStyle = au.muted; ctx.font = `500 ${rs * 0.8}px "IBM Plex Mono", ui-monospace, monospace`; ctx.textAlign = 'right';
    ctx.fillText(`${fmtClock(t)} / ${fmtClock(TL.total)}`, P.x + P.w, P.y - rs * 0.6);
  }
  if (au.source.trim()) { ctx.fillStyle = au.muted; ctx.font = `500 ${rs * 0.8}px ${stack}`; ctx.textAlign = 'left'; ctx.fillText(au.source.trim(), G.prog.x, G.prog.y - rs * 0.6); }
  ctx.restore();
}

/* ---------- preview ---------- */
function syncAudio(t, playing) {
  const el = media?.el; if (!el) return;
  const target = trimIn + t;
  if (playing && t < auTL.total) {
    if (el.paused) { el.currentTime = target; el.play().catch(() => {}); }
    else if (Math.abs(el.currentTime - target) > 0.25) el.currentTime = target;
  } else if (!el.paused) el.pause();
}
function auPaint(t) {
  if (!auTL) return;
  const { W, H } = dims(au.out);
  if (auCanvas.width !== W || auCanvas.height !== H) { auCanvas.width = W; auCanvas.height = H; }
  syncAudio(t, auPlayer.playing);
  const ctx = auCanvas.getContext('2d');
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  auDraw(ctx, W, H, t, auTL);
  const g = auGuides.getContext('2d');
  const dpr = window.devicePixelRatio || 1;
  const gw = Math.round(auViewer.clientWidth * dpr), gh = Math.round(auViewer.clientHeight * dpr);
  if (auGuides.width !== gw || auGuides.height !== gh) { auGuides.width = gw; auGuides.height = gh; }
  g.clearRect(0, 0, gw, gh);
  if (auPlayer.guides) drawGuides(g, gw, gh, au.out.aspect, gw / W * 2);
}
function auRenderTrack() {
  const tr = $('#au-track'), T = auTL.total;
  let html = '';
  auTL.cues.forEach((c, k) => { html += `<div class="blk${k % 2 ? ' dim' : ''}" style="left:${c.t0 / T * 100}%;width:${Math.max(0.3, (Math.min(c.t1, T) - c.t0) / T * 100)}%;--c:${au.hiWord}"></div>`; });
  tr.innerHTML = html + '<div class="head"></div>';
}
function auRefresh() {
  const { cues, timed } = auCues();
  auTL = { cues, timed, total: clipLen() };
  const { OW, OH } = dims(au.out);
  $('#au-fmt').textContent = `${OW} × ${OH} · ${fmtClock(auTL.total)} · ${cues.length} caption${cues.length === 1 ? '' : 's'} · ${au.out.fps} fps`;
  $('#au-cap-note').textContent = !au.captions.trim() ? 'No captions.'
    : timed ? `Timed from your caption file${media && trimIn ? `, shifted by the ${fmtSec(trimIn)} start` : ''}.`
    : media ? 'Spread over the parts where someone is talking. For exact timing, import an .srt or .vtt file.' : 'Spread over the sample. Choose your audio to time them to it.';
  $('#au-sample-tag').hidden = !!media;
  $('#au-trim-f').hidden = !media;
  auRenderTrack();
  if (auPlayer.t > auTL.total) auPlayer.seek(auTL.total); else { auPaint(auPlayer.t); auPlayer.sync(); }
  saveSoon('au', au);
}
function auFit() { fitViewer(auMonitor, auViewer, au.out.aspect); auPaint(auPlayer.t); }
async function auEnsureFonts() {
  try { await Promise.all([500, 700, 800].map(w => document.fonts.load(`${w} 40px ${FONTS[au.font]}`, 'Ag'))); } catch (e) { /* fall back silently */ }
}

/* ---------- loading audio ---------- */
async function openAudio(file) {
  const st = $('#au-status');
  st.className = 'status';
  if (!/^(audio|video)\//.test(file.type) && !/\.(mp3|m4a|aac|wav|ogg|opus|flac|mp4|m4v|mov|webm)$/i.test(file.name)) { st.textContent = 'Choose an audio file (MP3, M4A, WAV) or a video with sound.'; st.className = 'status err'; return; }
  if (file.size > MAX_BYTES) { st.textContent = 'That file is over 300 MB. Trim the part you need in an editor first, then open the shorter file.'; st.className = 'status err'; return; }
  st.textContent = `Reading ${file.name}…`;
  try {
    const buf = await new OfflineAudioContext(2, 1, 48000).decodeAudioData(await file.arrayBuffer());
    const env = envelope(buf);
    if (media) { media.el.pause(); URL.revokeObjectURL(media.url); media.el.remove(); }
    const url = URL.createObjectURL(file);
    const el = document.createElement('audio'); el.preload = 'auto'; el.src = url;
    $('#au-media').appendChild(el);
    media = { name: file.name, buf, env, voiced: voicedOf(env), url, el };
    trimIn = 0;
    const max = Math.max(0, buf.duration - 0.5);
    $('#au-trim').max = max.toFixed(2); $('#au-trim').value = 0; $('#au-trim-out').textContent = fmtSec(0);
    $('#au-len').max = Math.min(MAX_LEN, Math.ceil(buf.duration)).toString();
    st.textContent = buf.duration > MAX_LEN ? `${file.name} · ${fmtClock(buf.duration)}. Clips are limited to 10 minutes; use Start at to pick the part you need.` : `${file.name} · ${fmtClock(buf.duration)}`;
    if (au.captions === AU_SAMPLE_CAPS) { au.captions = ''; $('#au-captions').value = ''; }
    if (au.name === AU_DEFAULT.name) { bindName.set(''); }
    if (au.role === AU_DEFAULT.role) { bindRole.set(''); }
    auRefresh(); auPlayer.seek(0);
  } catch (e) {
    st.textContent = `This browser can’t read the sound in “${file.name}”. Convert it to MP3 or WAV and try again.`; st.className = 'status err';
  }
}

/* ---------- controls ---------- */
const auChanged = key => {
  if (key === 'out.aspect') auFit();
  if (key === 'font') auEnsureFonts().then(auRefresh);
  auRefresh();
};
$('#au-pick').addEventListener('click', () => $('#au-file').click());
$('#au-file').addEventListener('change', e => { const f = e.target.files[0]; e.target.value = ''; if (f) openAudio(f); });
for (const el of [$('#au-drop'), auViewer]) {
  el.addEventListener('dragover', e => { if ([...e.dataTransfer.items].some(i => i.kind === 'file')) { e.preventDefault(); el.classList.add(el === auViewer ? 'dropping' : 'over'); } });
  el.addEventListener('dragleave', () => el.classList.remove('dropping', 'over'));
  el.addEventListener('drop', e => {
    e.preventDefault(); el.classList.remove('dropping', 'over');
    const f = e.dataTransfer.files[0]; if (!f) return;
    if (/\.(srt|vtt)$/i.test(f.name)) importCaptions(f); else if (/^image\//.test(f.type)) setPhoto(f); else openAudio(f);
  });
}
$('#au-trim').addEventListener('input', e => { trimIn = +e.target.value; $('#au-trim-out').textContent = fmtSec(trimIn); auRefresh(); });
bindRange('au-len', au, 'length', v => (+v ? fmtSec(v) : 'to the end'), auChanged);
const bindName = bindText('au-name', au, 'name', auChanged);
const bindRole = bindText('au-role', au, 'role', auChanged);
bindText('au-source', au, 'source', auChanged);
$('#au-captions').value = au.captions;
$('#au-captions').addEventListener('input', e => { au.captions = e.target.value; auRefresh(); });
async function importCaptions(f) {
  const text = await f.text();
  if (!text.includes('-->')) { toast('That file has no caption times in it. Paste the words instead, or export an .srt or .vtt.'); return; }
  au.captions = text; $('#au-captions').value = text; auRefresh();
  toast(`Imported ${parseTimed(text).length} captions from ${f.name}.`);
}
$('#au-capfile-pick').addEventListener('click', () => $('#au-capfile').click());
$('#au-capfile').addEventListener('change', e => { const f = e.target.files[0]; e.target.value = ''; if (f) importCaptions(f); });
async function setPhoto(f) {
  try { photo = await loadImageFile(f); $('#au-photo-clear').hidden = false; auRefresh(); } catch (err) { toast(err.message); }
}
$('#au-photo-pick').addEventListener('click', () => $('#au-photo-file').click());
$('#au-photo-file').addEventListener('change', e => { const f = e.target.files[0]; e.target.value = ''; if (f) setPhoto(f); });
$('#au-photo-clear').addEventListener('click', () => { photo = null; $('#au-photo-clear').hidden = true; auRefresh(); });
bindCheck('au-showphoto', au, 'showPhoto', auChanged);
bindSeg('au-capmode', au, 'capMode', auChanged);
bindRange('au-capwords', au, 'capWords', v => String(v), auChanged);
bindRange('au-capsize', au, 'capSize', v => v + 'px', auChanged);
bindSeg('au-cappos', au, 'capPos', auChanged);
bindCheck('au-capbox', au, 'capBox', auChanged);
bindCheck('au-upper', au, 'upper', auChanged);
bindSeg('au-style', au, 'style', auChanged);
bindCheck('au-progress', au, 'progress', auChanged);
bindColor('au-bg', au, 'bg', auChanged);
bindColor('au-fg', au, 'fg', auChanged);
bindColor('au-muted', au, 'muted', auChanged);
bindColor('au-wave', au, 'wave', auChanged);
bindColor('au-hiword', au, 'hiWord', auChanged);
bindSelect('au-font', au, 'font', auChanged);

const auPlayer = makePlayer('au', {
  total: () => (auTL ? auTL.total : 1), fps: () => au.out.fps, render: auPaint, track: $('#au-track'),
  onPause: () => media?.el.pause(),
  onLoop: () => media?.el.pause(),
});

/* ---------- export ---------- */
/** The trimmed stretch of the soundtrack, with 20 ms fades so the cut points don't click */
function sliceAudio() {
  const b = media.buf, sr = b.sampleRate;
  const a = Math.floor(trimIn * sr), n = Math.max(1, Math.min(b.length - a, Math.round(clipLen() * sr)));
  const out = new AudioBuffer({ length: n, numberOfChannels: b.numberOfChannels, sampleRate: sr });
  const fade = Math.min(Math.round(0.02 * sr), Math.floor(n / 2));
  for (let c = 0; c < b.numberOfChannels; c++) {
    const d = b.getChannelData(c).subarray(a, a + n).slice();
    for (let i = 0; i < fade; i++) { const g = i / fade; d[i] *= g; d[n - 1 - i] *= g; }
    out.copyToChannel(d, c);
  }
  return out;
}
async function auRenderOne() {
  auPlayer.pause();
  if (!media) { toast('Choose an audio file first. The sample has no sound to render.'); return 'error'; }
  await auEnsureFonts(); auRefresh();
  const TL = auTL, { q, W, H, OW, OH } = dims(au.out);
  return runRender({
    title: `Rendering ${fmtClock(TL.total)} audiogram`, OW, OH, fps: au.out.fps, total: TL.total, format: au.out.format,
    name: `audiogram_${slug(au.name || media.name.replace(/\.[^.]+$/, '')) || 'clip'}_${OW}x${OH}`,
    getAudio: async () => sliceAudio(),
    frame: (ctx, t) => { ctx.setTransform(q, 0, 0, q, 0, 0); auDraw(ctx, W, H, t, TL); },
  });
}
async function auStill() {
  await auEnsureFonts(); auRefresh();
  const { q, W, H, OW, OH } = dims(au.out);
  const c = document.createElement('canvas'); c.width = OW; c.height = OH;
  const ctx = c.getContext('2d'); ctx.setTransform(q, 0, 0, q, 0, 0);
  auDraw(ctx, W, H, auPlayer.t, auTL);
  await saveStill(c, `audiogram_frame_${OW}x${OH}.png`);
}
buildOutputPanel('au', au.out, auChanged, {
  render: auRenderOne,
  renderBoth: () => renderBoth(au.out, () => { auFit(); auRefresh(); }, auRenderOne),
  still: auStill,
});
buildPresetPanel('au', au, AU_PRESET_KEYS, AU_BUILTINS, () => { resync(au); resync(au.out); auEnsureFonts().then(auRefresh); auFit(); auRefresh(); });
new ResizeObserver(() => { if (!toolAu.hidden) auFit(); }).observe(auMonitor);

CS.tools.au = {
  id: 'au',
  player: auPlayer,
  boot() { auRefresh(); auPlayer.seek(2.2); auEnsureFonts().then(auRefresh); },
  show() { requestAnimationFrame(auFit); },
  hide() { auPlayer.pause(); },
  debug: {
    au, auPlayer, get auTL() { return auTL; }, get auMedia() { return media; },
    auCueAt: t => { const c = cueAt(auTL, t); return c ? c.words.map(w => w.text).join(' ') : null; },
  },
};
})();
