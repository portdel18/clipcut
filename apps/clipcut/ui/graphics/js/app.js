/* Clip Studio · app shell: tabs, help dialog, keyboard shortcut, environment check, boot.
   Each tool file registers CS.tools.<id> = { player, boot(), show(), hide(), debug }; layout.js keeps the
   order of tabs and sections. */
(() => {
'use strict';
const { $, store } = window.CS;
const TOOLS = ['ul', 'mp', 'sp', 'sz', 'tl', 'nm', 'au', 'lt', 'th'];
// Tabs in the top bar; a group is one tab holding several tools, with their own row of tabs under the top bar
const GROUPS = { as: ['lt', 'th'] };
const TABS = ['ul', 'mp', 'sp', 'sz', 'tl', 'nm', 'au', 'as'];
const tools = window.CS.tools, L = window.CS.layout;
const groupOf = id => Object.keys(GROUPS).find(g => GROUPS[g].includes(id));
let current = null;
// the host app (ClipCut) opens a tool to edit a graphic made with it
window.CS.showTool = id => showTab(id);

function showTab(id) {
  // A group opens on the tool you last used in it
  if (GROUPS[id]) { const items = L.visibleItems(id), last = store.get('tab.' + id); id = items.includes(last) ? last : items[0]; }
  const reachable = L.visibleTools();
  if (!reachable.includes(id)) id = reachable[0];
  const g = groupOf(id);
  for (const k of TOOLS) {
    const on = k === id;
    $('#tool-' + k).hidden = !on;
    if (!on) tools[k].hide();
  }
  for (const e of TABS) $('#tab-' + e).setAttribute('aria-selected', String(e === id || e === g));
  for (const [gid, items] of Object.entries(GROUPS)) {
    $('#subtabs-' + gid).hidden = gid !== g;
    for (const t of items) $('#tab-' + t).setAttribute('aria-selected', String(t === id));
    $('#tab-' + gid).setAttribute('aria-controls', 'tool-' + (gid === g ? id : L.visibleItems(gid)[0] || items[0]));
  }
  current = id;
  store.set('tab', id);
  if (g) store.set('tab.' + g, id);
  tools[id].show();
}
for (const e of new Set([...TABS, ...TOOLS])) $('#tab-' + e).addEventListener('click', () => showTab(e));
// Arrow keys move along the tab row that has focus: the top bar, or a group's own row
document.addEventListener('keydown', e => {
  if (e.key !== 'ArrowRight' && e.key !== 'ArrowLeft') return;
  const bar = e.target.closest?.('.tabs, .subtabs');
  if (!bar) return;
  const list = bar.matches('.tabs') ? L.visibleTabs() : L.visibleItems(bar.id.replace('subtabs-', ''));
  const cur = list.indexOf(e.target.id.slice(4));
  const next = list[(cur + (e.key === 'ArrowRight' ? 1 : list.length - 1)) % list.length];
  showTab(next); $('#tab-' + next).focus();
});
$('#open-help').addEventListener('click', () => $('#help-dlg').showModal());
$('#help-close').addEventListener('click', () => $('#help-dlg').close());

// Space bar plays/pauses the visible tool (ignored while typing or inside dialogs)
document.addEventListener('keydown', e => {
  if (e.code !== 'Space' || e.target.closest('input, textarea, select, button, [contenteditable]') || document.querySelector('dialog[open]')) return;
  e.preventDefault();
  tools[current]?.player?.toggle();   // Thumbnails has no player
});

// Warn when the page is opened somewhere that blocks the libraries (for example an in-app file preview)
(function envCheck() {
  const missing = [];
  if (!window.maplibregl) missing.push('the map engine');
  if (!window.Mp4Muxer) missing.push('the MP4 writer');
  if (!missing.length) return;
  const b = $('#env-banner');
  b.innerHTML = '<strong>Some parts didn’t load</strong> (' + missing.join(' and ') + '). This happens inside an app’s file preview or without internet. Open the page in Chrome or Edge, from your computer or from its GitHub Pages address.';
  b.hidden = false;
})();

for (const k of TOOLS) tools[k].boot();
L.init({ tabs: TABS, groups: GROUPS, onChange: () => showTab(current) });
showTab(store.get('tab', 'ul'));

// Handle for tests and debugging in the browser console
const dbg = {};
for (const k of TOOLS) Object.defineProperties(dbg, Object.getOwnPropertyDescriptors(tools[k].debug || {}));
Object.defineProperties(dbg, Object.getOwnPropertyDescriptors(L.debug));
window.__clipStudio = dbg;
})();
