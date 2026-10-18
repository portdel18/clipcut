/* ClipCut · the one Clip Studio helper that graphics/js/platforms.js needs, so the player can draw what
   TikTok, Reels and Shorts cover (the same drawing as in the Graphics tab). */
window.CS = window.CS || {};
window.CS.roundRect = window.CS.roundRect || function roundRect(ctx, x, y, w, h, r) {
  r = Math.max(0, Math.min(r, w / 2, h / 2));
  ctx.beginPath();
  ctx.moveTo(x + r, y);
  ctx.arcTo(x + w, y, x + w, y + h, r);
  ctx.arcTo(x + w, y + h, x, y + h, r);
  ctx.arcTo(x, y + h, x, y, r);
  ctx.arcTo(x, y, x + w, y, r);
  ctx.closePath();
};
