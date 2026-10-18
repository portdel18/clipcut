/* Clip Studio · platform overlays: what TikTok, Instagram Reels, and YouTube Shorts draw on top of a 9:16 video,
   so text and faces stay clear of it. Preview only: drawn on each tool's guide canvas, never in a render.
   Classic script (not an ES module) so index.html still works when opened straight from disk.

   Every layout is in a 1080 × 1920 frame. The covered areas (tinted red) and the clear area (amber box) follow the
   platforms' own safe-zone guidance as of 2026; the icons and text are stand-ins, close in size and place to the
   real ones, not copies of them. */
(() => {
'use strict';
const { roundRect } = window.CS;

const PLATFORMS = {
  tiktok: {
    name: 'TikTok',
    covered: [[0, 0, 1080, 230], [930, 790, 150, 940], [0, 1500, 930, 280], [0, 1780, 1080, 140]],
    clear: [60, 230, 860, 1270],
    draw: drawTikTok,
  },
  reels: {
    name: 'Instagram Reels',
    covered: [[0, 0, 1080, 260], [940, 1080, 140, 690], [0, 1490, 940, 280], [0, 1770, 1080, 150]],
    clear: [64, 260, 860, 1230],
    draw: drawReels,
  },
  shorts: {
    name: 'YouTube Shorts',
    covered: [[0, 0, 1080, 220], [930, 900, 150, 870], [0, 1480, 930, 290], [0, 1770, 1080, 150]],
    clear: [60, 220, 860, 1260],
    draw: drawShorts,
  },
};

/* ---------- drawing helpers (1080 × 1920 units) ---------- */
let g = null;
const WHITE = 'rgba(255,255,255,0.95)';
function shadowOn() { g.shadowColor = 'rgba(0,0,0,0.45)'; g.shadowBlur = 8; g.shadowOffsetY = 2; }
function shadowOff() { g.shadowColor = 'transparent'; g.shadowBlur = 0; g.shadowOffsetY = 0; }
function text(str, x, y, size, { weight = 600, align = 'left', color = WHITE } = {}) {
  g.font = `${weight} ${size}px "Helvetica Neue", Arial, sans-serif`;
  g.textAlign = align; g.textBaseline = 'alphabetic'; g.fillStyle = color;
  g.fillText(str, x, y);
}
function stroke(w = 6) { g.lineWidth = w; g.strokeStyle = WHITE; g.lineJoin = 'round'; g.lineCap = 'round'; g.stroke(); }
function fill(c = WHITE) { g.fillStyle = c; g.fill(); }
function heart(cx, cy, r) {
  g.beginPath();
  g.moveTo(cx, cy + r * 0.9);
  g.bezierCurveTo(cx - r * 1.6, cy - r * 0.1, cx - r * 0.75, cy - r * 1.25, cx, cy - r * 0.45);
  g.bezierCurveTo(cx + r * 0.75, cy - r * 1.25, cx + r * 1.6, cy - r * 0.1, cx, cy + r * 0.9);
  g.closePath();
}
function bubble(cx, cy, r) {
  g.beginPath();
  g.ellipse(cx, cy - r * 0.05, r, r * 0.86, 0, Math.PI * 0.62, Math.PI * 2.5);
  g.lineTo(cx - r * 0.95, cy + r * 0.85);
  g.closePath();
}
function bookmark(cx, cy, r) {
  g.beginPath();
  g.moveTo(cx - r * 0.7, cy - r); g.lineTo(cx + r * 0.7, cy - r); g.lineTo(cx + r * 0.7, cy + r);
  g.lineTo(cx, cy + r * 0.5); g.lineTo(cx - r * 0.7, cy + r); g.closePath();
}
function shareArrow(cx, cy, r) {
  g.beginPath();
  g.moveTo(cx + r, cy - r * 0.1); g.lineTo(cx + r * 0.05, cy - r * 0.9); g.lineTo(cx + r * 0.05, cy - r * 0.45);
  g.quadraticCurveTo(cx - r * 1.0, cy - r * 0.35, cx - r, cy + r * 0.85);
  g.quadraticCurveTo(cx - r * 0.45, cy + r * 0.15, cx + r * 0.05, cy + r * 0.2);
  g.lineTo(cx + r * 0.05, cy + r * 0.7); g.closePath();
}
function plane(cx, cy, r) {
  g.beginPath();
  g.moveTo(cx - r, cy - r * 0.75); g.lineTo(cx + r, cy - r * 0.75); g.lineTo(cx - r * 0.1, cy + r);
  g.lineTo(cx - r * 0.3, cy - r * 0.1); g.closePath();
  g.moveTo(cx - r * 0.3, cy - r * 0.1); g.lineTo(cx + r, cy - r * 0.75);
}
function thumb(cx, cy, r, down) {
  g.save(); g.translate(cx, cy); if (down) g.rotate(Math.PI);
  g.beginPath();
  g.moveTo(-r, -r * 0.1); g.lineTo(-r * 0.55, -r * 0.1); g.lineTo(-r * 0.55, r); g.lineTo(-r, r); g.closePath();
  g.moveTo(-r * 0.35, -r * 0.1); g.lineTo(-r * 0.05, -r * 0.95); g.quadraticCurveTo(r * 0.25, -r * 0.95, r * 0.15, -r * 0.35);
  g.lineTo(r * 0.85, -r * 0.35); g.quadraticCurveTo(r * 1.05, -r * 0.3, r * 0.95, -r * 0.05);
  g.lineTo(r * 0.7, r * 0.85); g.quadraticCurveTo(r * 0.62, r, r * 0.45, r); g.lineTo(-r * 0.35, r); g.closePath();
  g.restore();
}
function avatar(cx, cy, r) {
  g.beginPath(); g.arc(cx, cy, r, 0, Math.PI * 2); fill('rgba(205,205,210,0.9)');
  g.beginPath(); g.arc(cx, cy - r * 0.18, r * 0.36, 0, Math.PI * 2); fill('rgba(120,120,128,0.95)');
  g.beginPath(); g.ellipse(cx, cy + r * 0.62, r * 0.62, r * 0.42, 0, Math.PI, 0); fill('rgba(120,120,128,0.95)');
  g.beginPath(); g.arc(cx, cy, r, 0, Math.PI * 2); g.lineWidth = 3; g.strokeStyle = WHITE; g.stroke();
}
function statusBar() {
  text('9:41', 70, 66, 38, { weight: 700 });
  g.beginPath(); roundRect(g, 940, 40, 62, 30, 8); stroke(3);
  g.beginPath(); roundRect(g, 945, 45, 44, 20, 4); fill();
  for (let i = 0; i < 4; i++) { g.beginPath(); roundRect(g, 846 + i * 16, 64 - (i + 1) * 7, 10, (i + 1) * 7, 2); fill(); }
}
function navBar(y, items) {
  g.fillStyle = 'rgba(0,0,0,0.88)'; g.fillRect(0, y, 1080, 1920 - y);
  items.forEach((it, i) => {
    const cx = 108 + i * 216, cy = y + 52;
    if (it === '+') {
      g.beginPath(); roundRect(g, cx - 46, cy - 30, 92, 60, 14); fill('#fff');
      text('+', cx, cy + 16, 50, { weight: 700, align: 'center', color: '#111' });
    } else {
      g.beginPath(); roundRect(g, cx - 22, cy - 24, 44, 44, 10); stroke(5);
      if (it) text(it, cx, y + 118, 26, { weight: 600, align: 'center' });
    }
  });
}
function rail(x, items) {
  for (const it of items) {
    shadowOn();
    if (it.kind === 'avatar') {
      avatar(x, it.y, 50);
      if (it.plus) { g.beginPath(); g.arc(x, it.y + 52, 18, 0, Math.PI * 2); fill(it.plus); text('+', x, it.y + 64, 32, { weight: 700, align: 'center' }); }
    } else if (it.kind === 'disc') {
      g.beginPath(); g.arc(x, it.y, 46, 0, Math.PI * 2); fill('rgba(40,40,44,0.95)');
      g.beginPath(); g.arc(x, it.y, 22, 0, Math.PI * 2); fill('rgba(205,205,210,0.9)');
    } else if (it.kind === 'square') {
      g.beginPath(); roundRect(g, x - 34, it.y - 34, 68, 68, 12); fill('rgba(205,205,210,0.9)');
      g.beginPath(); roundRect(g, x - 34, it.y - 34, 68, 68, 12); stroke(4);
    } else if (it.kind === 'dots') {
      for (const dx of [-22, 0, 22]) { g.beginPath(); g.arc(x + dx, it.y, 6, 0, Math.PI * 2); fill(); }
    } else {
      ({ heart, bubble, bookmark, share: shareArrow, plane, like: (a, b, c) => thumb(a, b, c), dislike: (a, b, c) => thumb(a, b, c, true) })[it.kind](x, it.y, it.r || 36);
      if (it.solid) fill(); else stroke(6);
    }
    if (it.label) text(it.label, x, it.y + (it.kind === 'avatar' ? 0 : 82), 28, { weight: 600, align: 'center' });
    shadowOff();
  }
}
function bottomShade(from) {
  const gr = g.createLinearGradient(0, from, 0, 1920);
  gr.addColorStop(0, 'rgba(0,0,0,0)'); gr.addColorStop(1, 'rgba(0,0,0,0.5)');
  g.fillStyle = gr; g.fillRect(0, from, 1080, 1920 - from);
}

/* ---------- the three platforms ---------- */
function drawTikTok() {
  bottomShade(1250);
  shadowOn();
  statusBar();
  text('LIVE', 70, 178, 30, { weight: 800 });
  text('Following', 430, 178, 40, { weight: 600, align: 'center', color: 'rgba(255,255,255,0.7)' });
  text('For You', 640, 178, 40, { weight: 800, align: 'center' });
  g.fillStyle = WHITE; g.fillRect(606, 196, 68, 6);
  g.beginPath(); g.arc(1000, 162, 22, 0, Math.PI * 2); stroke(6);
  g.beginPath(); g.moveTo(1016, 178); g.lineTo(1034, 196); stroke(6);
  shadowOff();
  rail(1000, [
    { kind: 'avatar', y: 880, plus: '#fe2c55' },
    { kind: 'heart', y: 1070, solid: true, label: '248K' },
    { kind: 'bubble', y: 1215, solid: true, label: '1,204' },
    { kind: 'bookmark', y: 1360, solid: true, label: '8,912' },
    { kind: 'share', y: 1505, solid: true, label: '3,310' },
    { kind: 'disc', y: 1665 },
  ]);
  shadowOn();
  text('@yournewsroom', 40, 1580, 40, { weight: 700 });
  text('Your caption sits here and can run to', 40, 1636, 36, { weight: 400 });
  text('two lines before it cuts off… more', 40, 1682, 36, { weight: 400 });
  text('♪  original sound - yournewsroom', 40, 1740, 32, { weight: 500 });
  shadowOff();
  navBar(1780, ['Home', 'Friends', '+', 'Inbox', 'Profile']);
}
function drawReels() {
  bottomShade(1250);
  shadowOn();
  statusBar();
  text('Reels', 40, 196, 54, { weight: 800 });
  g.beginPath(); roundRect(g, 976, 150, 64, 48, 12); stroke(6);
  g.beginPath(); g.arc(1008, 174, 13, 0, Math.PI * 2); stroke(5);
  shadowOff();
  rail(1000, [
    { kind: 'heart', y: 1150, label: '48.2K' },
    { kind: 'bubble', y: 1300, label: '932' },
    { kind: 'plane', y: 1450, label: '1,203' },
    { kind: 'dots', y: 1580 },
    { kind: 'square', y: 1700 },
  ]);
  shadowOn();
  avatar(78, 1556, 36);
  text('yournewsroom', 132, 1570, 38, { weight: 700 });
  g.beginPath(); roundRect(g, 420, 1528, 132, 58, 14); stroke(3);
  text('Follow', 486, 1568, 30, { weight: 700, align: 'center' });
  text('Your caption sits here… more', 40, 1646, 34, { weight: 400 });
  text('♪  yournewsroom · Original audio', 40, 1710, 30, { weight: 500 });
  shadowOff();
  navBar(1770, ['', '', '+', '', '']);
}
function drawShorts() {
  bottomShade(1250);
  shadowOn();
  statusBar();
  g.beginPath(); g.arc(850, 150, 22, 0, Math.PI * 2); stroke(6);
  g.beginPath(); g.moveTo(866, 166); g.lineTo(884, 184); stroke(6);
  g.beginPath(); roundRect(g, 912, 128, 58, 44, 10); stroke(5);
  for (const dy of [-20, 0, 20]) { g.beginPath(); g.arc(1030, 152 + dy, 5, 0, Math.PI * 2); fill(); }
  shadowOff();
  rail(1000, [
    { kind: 'like', y: 990, solid: true, label: '52K' },
    { kind: 'dislike', y: 1130, solid: true, label: 'Dislike' },
    { kind: 'bubble', y: 1270, solid: true, label: '1.1K' },
    { kind: 'share', y: 1410, solid: true, label: 'Share' },
    { kind: 'dots', y: 1560 },
    { kind: 'square', y: 1690 },
  ]);
  shadowOn();
  avatar(78, 1560, 36);
  text('@yournewsroom', 132, 1574, 36, { weight: 700 });
  g.beginPath(); roundRect(g, 450, 1530, 190, 60, 30); fill('#fff');
  text('Subscribe', 545, 1571, 30, { weight: 700, align: 'center', color: '#0f0f0f' });
  text('Your video title sits here', 40, 1652, 36, { weight: 500 });
  text('♪  original sound', 40, 1716, 30, { weight: 500 });
  shadowOff();
  navBar(1770, ['Home', 'Shorts', '+', 'Subs', 'You']);
}

/** Draws a platform's interface over a 9:16 preview of size W × H (device pixels). */
function drawPlatformUI(ctx, W, H, kind) {
  const P = PLATFORMS[kind];
  if (!P) return;
  g = ctx;
  ctx.save();
  ctx.scale(W / 1080, H / 1920);
  // What the interface covers, then the interface itself, then the area that stays clear
  ctx.fillStyle = 'rgba(255,75,62,0.16)';
  for (const [x, y, w, h] of P.covered) ctx.fillRect(x, y, w, h);
  P.draw();
  const [cx, cy, cw, ch] = P.clear;
  ctx.setLineDash([18, 12]); ctx.lineWidth = 4; ctx.strokeStyle = 'rgba(244,196,67,0.95)';
  ctx.strokeRect(cx, cy, cw, ch);
  ctx.setLineDash([]);
  ctx.font = '700 28px Barlow, Arial, sans-serif'; ctx.textAlign = 'left'; ctx.textBaseline = 'alphabetic';
  const tag = `Clear of the ${P.name} interface · preview only`;
  const tw = ctx.measureText(tag).width;
  ctx.fillStyle = 'rgba(0,0,0,0.6)'; ctx.fillRect(cx, cy, tw + 24, 44);
  ctx.fillStyle = 'rgba(244,196,67,0.95)'; ctx.fillText(tag, cx + 12, cy + 31);
  ctx.restore();
  g = null;
}

Object.assign(window.CS, { PLATFORMS, drawPlatformUI });
})();
