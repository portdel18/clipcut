/* Clip Studio · colour dropper: a button beside every colour box that takes a colour from a
   picture on the page (a tool's preview, a photo, and in ClipCut the player and the timeline's
   pictures). Click it, then click the colour; Esc stops.
   The colour pop-up's own dropper doesn't reach the page inside a desktop app's web view (ClipCut
   on Windows), so this one reads the pixels itself. In a browser with the EyeDropper API, and not
   inside an app, that is used instead (it reads anywhere on the screen).
   With `data-colour-hex` on <html>, colour boxes that have no hex box get one (ClipCut; Clip
   Studio's own boxes have theirs). No dependencies, so ClipCut loads it from its copy of the page.
   Classic script (not an ES module) so index.html still works when opened straight from disk. */
(() => {
'use strict';
const ICON = '<svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M15 4.5a2.1 2.1 0 0 1 3 0l1.5 1.5a2.1 2.1 0 0 1 0 3L17 11.5 12.5 7z"/><path d="M13.5 8L5.5 16 5 19l3-.5 8-8"/></svg>';
const TIP = 'Pick a colour from a picture: click this, then click the colour (Esc: stop)';
let target = null, button = null;

const inApp = () => {
  if (window.__clipcutHost) return true;
  try { return window.parent !== window && !!window.parent.__clipcutHost; } catch (_) { return false; }
};
const hex2 = v => Math.round(Math.min(255, Math.max(0, v))).toString(16).padStart(2, '0');
const say = msg => {
  const t = (window.CS && window.CS.toast) || window.toast;
  if (typeof t === 'function') t(msg);
};
/** `#rgb`, `rgb`, `#rrggbb` or `rrggbb` as `#rrggbb`; null for anything else. */
function parseHex(v) {
  const s = String(v || '').trim().replace(/^#/, '').toLowerCase();
  if (/^[0-9a-f]{6}$/.test(s)) return '#' + s;
  if (/^[0-9a-f]{3}$/.test(s)) return '#' + s.split('').map(c => c + c).join('');
  return null;
}
/** Sets colour box `input` to `v` as if it had been picked there (its own handlers send it). */
function setColour(input, v) {
  input.value = v;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

/** The colour shown at (x, y): the topmost picture there that isn't see-through, averaged over
 *  3 × 3 of its pixels. Null when no picture is there. */
function sample(x, y) {
  for (const el of document.elementsFromPoint(x, y)) {
    if (el.closest('[data-nopick]')) continue;
    const pic = el instanceof HTMLCanvasElement || el instanceof HTMLImageElement || el instanceof HTMLVideoElement;
    if (!pic) continue;
    const w = el.videoWidth || el.naturalWidth || el.width, h = el.videoHeight || el.naturalHeight || el.height;
    const r = el.getBoundingClientRect();
    if (!w || !h || !r.width || !r.height) continue;
    const sx = Math.floor((x - r.left) * w / r.width), sy = Math.floor((y - r.top) * h / r.height);
    try {
      const c = document.createElement('canvas');
      c.width = c.height = 3;
      const g = c.getContext('2d', { willReadFrequently: true });
      g.drawImage(el, sx - 1, sy - 1, 3, 3, 0, 0, 3, 3);
      const d = g.getImageData(0, 0, 3, 3).data;
      let rr = 0, gg = 0, bb = 0, a = 0;
      for (let i = 0; i < d.length; i += 4) { const k = d[i + 3]; rr += d[i] * k; gg += d[i + 1] * k; bb += d[i + 2] * k; a += k; }
      // see-through here: what is under it
      if (a < 9 * 8) continue;
      return '#' + hex2(rr / a) + hex2(gg / a) + hex2(bb / a);
    } catch (_) { /* a picture the page may not read: the next one under it */ }
  }
  return null;
}

function stop() {
  if (button) button.setAttribute('aria-pressed', 'false');
  target = button = null;
  document.documentElement.classList.remove('dropping');
  window.removeEventListener('pointerdown', down, true);
  window.removeEventListener('keydown', key, true);
}
function swallowClick() {
  const eat = e => { e.preventDefault(); e.stopPropagation(); };
  window.addEventListener('click', eat, { capture: true, once: true });
  // a click that never comes (a drag) mustn't eat a later one
  setTimeout(() => window.removeEventListener('click', eat, true), 600);
}
function down(e) {
  if (e.button !== 0 || (e.target.closest && e.target.closest('.dropper'))) return;
  e.preventDefault();
  e.stopPropagation();
  swallowClick();
  const input = target, v = sample(e.clientX, e.clientY);
  stop();
  if (!input) return;
  if (v) setColour(input, v);
  else say('There is no picture there. Press the dropper again, then click a colour in a picture.');
}
function key(e) {
  if (e.key !== 'Escape') return;
  e.preventDefault();
  e.stopPropagation();
  stop();
}
function start(input, b) {
  stop();
  if (window.EyeDropper && !inApp()) {
    new window.EyeDropper().open().then(r => { if (r && r.sRGBHex) setColour(input, parseHex(r.sRGBHex) || r.sRGBHex); }).catch(() => {});
    return;
  }
  target = input;
  button = b;
  b.setAttribute('aria-pressed', 'true');
  document.documentElement.classList.add('dropping');
  window.addEventListener('pointerdown', down, true);
  window.addEventListener('keydown', key, true);
  say('Click the colour you want in a picture: the preview, a clip, a photo (Esc: stop)');
}

/** A hex box for colour box `input`, kept in step with it (also when a script sets its value). */
function addHex(input) {
  const box = document.createElement('input');
  box.type = 'text';
  box.className = 'colour-hex';
  box.maxLength = 7;
  box.spellcheck = false;
  box.setAttribute('aria-label', (input.getAttribute('aria-label') || input.title || 'Colour') + ' as a hex code');
  box.title = 'Hex code, like #FFCC00';
  const show = () => { if (document.activeElement !== box) box.value = String(input.value || '').toUpperCase(); };
  const own = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value');
  if (own && own.get && own.set) {
    Object.defineProperty(input, 'value', { configurable: true, get() { return own.get.call(this); }, set(v) { own.set.call(this, v); show(); } });
  }
  input.addEventListener('input', show);
  const commit = () => {
    const v = parseHex(box.value);
    if (v && v !== String(input.value).toLowerCase()) setColour(input, v);
    box.value = String(input.value || '').toUpperCase();
  };
  box.addEventListener('input', () => { const v = box.value.trim().replace(/^#/, ''); if (/^[0-9a-f]{6}$/i.test(v)) commit(); });
  box.addEventListener('keydown', e => {
    // typing a code is not a shortcut
    e.stopPropagation();
    if (e.key === 'Enter') { e.preventDefault(); commit(); box.blur(); }
    if (e.key === 'Escape') { box.value = String(input.value || '').toUpperCase(); box.blur(); }
  });
  box.addEventListener('blur', commit);
  show();
  input.after(box);
  return box;
}

function add(input) {
  if (input.dataset.dropper) return;
  input.dataset.dropper = '1';
  let after = input;
  const next = input.nextElementSibling;
  // its hex box (Clip Studio's `<id>-hex`, or one this made), not some other text box after it
  if (next && next.matches('input[type="text"][maxlength="7"], .colour-hex')) after = next;
  else if (document.documentElement.hasAttribute('data-colour-hex')) after = addHex(input);
  const b = document.createElement('button');
  b.type = 'button';
  b.className = 'dropper';
  b.title = TIP;
  b.setAttribute('aria-label', 'Pick a colour from a picture');
  b.setAttribute('aria-pressed', 'false');
  b.innerHTML = ICON;
  b.addEventListener('click', e => {
    e.preventDefault();
    e.stopPropagation();
    if (target === input) stop(); else start(input, b);
  });
  after.after(b);
}
function scan(root) {
  if (root.matches && root.matches('input[type="color"]')) add(root);
  if (root.querySelectorAll) root.querySelectorAll('input[type="color"]').forEach(add);
}
const go = () => {
  scan(document.body);
  new MutationObserver(ms => { for (const m of ms) for (const n of m.addedNodes) if (n.nodeType === 1) scan(n); })
    .observe(document.body, { childList: true, subtree: true });
};
if (document.body) go(); else document.addEventListener('DOMContentLoaded', go, { once: true });
window.CSDropper = { sample, parseHex, stop };
})();
