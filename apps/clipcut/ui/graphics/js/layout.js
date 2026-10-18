/* Clip Studio · layout: your own order of tabs and of the settings sections under each tab, which ones show,
   and which sections start open. Changes save as you go; named layouts can be saved, switched, exported, and
   imported. Classic script (not an ES module) so index.html still works when opened straight from disk.

   app.js calls CS.layout.init({ tabs, groups, onChange }) and asks it which tabs are visible and in what order.
   A layout is { tabs: [entry], groups: { g: [tool] }, hidden: [entry or tool], secs: { tool: { order, hidden, open } } },
   where an entry is a tool id or a group id (a tab that holds several tools, such as Create Assets). */
(() => {
'use strict';
const { $, $$, store, clone, toast, downloadBlob } = window.CS;

let DEFAULT = null, L = null, onChange = () => {}, editing = false;
const saved = (() => { const v = store.get('layouts', {}); return v && typeof v === 'object' ? v : {}; })();
const labels = {};   // tab id → label, read from the buttons
const secKey = s => s.toLowerCase().replace(/&/g, 'and').replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');

/* ---------- state ---------- */
/** Saved order first (only ids that still exist), then anything new at its default position. */
function mergeOrder(savedOrder, def) {
  const out = (Array.isArray(savedOrder) ? savedOrder : []).filter((k, i, a) => def.includes(k) && a.indexOf(k) === i);
  def.forEach((k, i) => { if (!out.includes(k)) out.splice(Math.min(i, out.length), 0, k); });
  return out;
}
function normalize(v) {
  v = v && typeof v === 'object' ? v : {};
  const n = { tabs: mergeOrder(v.tabs, DEFAULT.tabs), groups: {}, hidden: [], secs: {} };
  for (const g of Object.keys(DEFAULT.groups)) n.groups[g] = mergeOrder(v.groups?.[g], DEFAULT.groups[g]);
  const known = [...DEFAULT.tabs, ...Object.values(DEFAULT.groups).flat()];
  n.hidden = (Array.isArray(v.hidden) ? v.hidden : []).filter(k => known.includes(k));
  for (const [tool, def] of Object.entries(DEFAULT.secs)) {
    const s = v.secs?.[tool] || {};
    n.secs[tool] = {
      order: mergeOrder(s.order, def),
      hidden: (Array.isArray(s.hidden) ? s.hidden : []).filter(k => def.includes(k)),
      open: Object.fromEntries(Object.entries(s.open && typeof s.open === 'object' ? s.open : {}).filter(([k]) => def.includes(k)).map(([k, o]) => [k, !!o])),
    };
  }
  if (!visibleTabs(n).length) n.hidden = [];
  return n;
}
const isGroup = id => id in DEFAULT.groups;
function visibleItems(g, l = L) { return l.groups[g].filter(t => !l.hidden.includes(t)); }
function visibleTabs(l = L) { return l.tabs.filter(e => !l.hidden.includes(e) && (!isGroup(e) || visibleItems(e, l).length)); }
/** Every tool you can reach, in tab order. */
function visibleTools() { return visibleTabs().flatMap(e => (isGroup(e) ? visibleItems(e) : [e])); }
function save() { store.set('layout', L); }
function changed() { save(); apply(); onChange(); }

/* ---------- applying a layout to the page ---------- */
function apply() {
  const bar = $('.tabs');
  for (const e of L.tabs) { const b = $('#tab-' + e); bar.appendChild(b); b.hidden = !visibleTabs().includes(e); }
  for (const g of Object.keys(L.groups)) {
    const sub = $('#subtabs-' + g);
    for (const t of L.groups[g]) { const b = $('#tab-' + t); sub.appendChild(b); b.hidden = L.hidden.includes(t); }
  }
  for (const [tool, s] of Object.entries(L.secs)) {
    const rack = $(`#tool-${tool} .rack`);
    for (const k of s.order) {
      const d = $(`:scope > details.sec[data-sec="${k}"]`, rack);
      rack.appendChild(d);
      d.classList.toggle('lay-hidden', s.hidden.includes(k));
      if (k in s.open && d.open !== s.open[k]) d.open = s.open[k];
      const eye = $('.sec-eye', d);
      eye.setAttribute('aria-pressed', String(!s.hidden.includes(k)));
      eye.setAttribute('aria-label', s.hidden.includes(k) ? 'Show this section' : 'Hide this section');
    }
  }
  if (editing) renderChips();
  fillSaved();
}

/* ---------- moving and hiding ---------- */
function move(list, id, to) {
  const i = list.indexOf(id);
  to = Math.max(0, Math.min(list.length - 1, to));
  if (i < 0 || i === to) return false;
  list.splice(i, 1); list.splice(to, 0, id);
  return true;
}
function listOf(id) {
  if (L.tabs.includes(id)) return L.tabs;
  return Object.values(L.groups).find(a => a.includes(id));
}
function setTabHidden(id, hide) {
  const was = L.hidden.slice();
  L.hidden = L.hidden.filter(k => k !== id);
  if (hide) L.hidden.push(id);
  if (!visibleTabs().length) { L.hidden = was; toast('Keep at least one tab showing.'); return false; }
  changed();
  return true;
}
function moveSec(tool, key, to) { if (move(L.secs[tool].order, key, to)) changed(); }
function setSecHidden(tool, key, hide) {
  const s = L.secs[tool];
  s.hidden = s.hidden.filter(k => k !== key);
  if (hide) s.hidden.push(key);
  changed();
}

/* ---------- edit mode ---------- */
function setEditing(on) {
  editing = on;
  document.body.classList.toggle('editing-layout', on);
  $('#layout-bar').hidden = !on;
  const btn = $('#layout-edit');
  btn.setAttribute('aria-pressed', String(on));
  btn.textContent = on ? 'Done' : 'Edit layout';
  if (on) renderChips();
}
/** The tab list in the layout bar: a checkbox to show or hide each tab, and arrows to move it. */
function renderChips() {
  const host = $('#layout-tabs');
  const chip = (id, list, sub) => {
    const i = list.indexOf(id), shown = !L.hidden.includes(id);
    return `<span class="lchip${sub ? ' sub' : ''}${shown ? '' : ' off'}" data-id="${id}">
      <label><input type="checkbox" data-act="show"${shown ? ' checked' : ''}> ${labels[id]}</label>
      <button type="button" class="lmv" data-act="left" aria-label="Move ${labels[id]} ${sub ? 'up' : 'left'}"${i === 0 ? ' disabled' : ''}>‹</button>
      <button type="button" class="lmv" data-act="right" aria-label="Move ${labels[id]} ${sub ? 'down' : 'right'}"${i === list.length - 1 ? ' disabled' : ''}>›</button>
    </span>`;
  };
  host.innerHTML = L.tabs.map(e => (isGroup(e)
    ? `<span class="lgroup">${chip(e, L.tabs)}<span class="lgroup-items" aria-label="${labels[e]}">${L.groups[e].map(t => chip(t, L.groups[e], true)).join('')}</span></span>`
    : chip(e, L.tabs))).join('');
}
$('#layout-tabs').addEventListener('click', e => {
  const b = e.target.closest('button[data-act]');
  if (!b) return;
  const id = b.closest('.lchip').dataset.id, list = listOf(id);
  if (move(list, id, list.indexOf(id) + (b.dataset.act === 'left' ? -1 : 1))) {
    changed();
    $(`#layout-tabs .lchip[data-id="${id}"] button[data-act="${b.dataset.act}"]`)?.focus();
  }
});
$('#layout-tabs').addEventListener('change', e => {
  const c = e.target.closest('input[data-act="show"]');
  if (!c) return;
  if (!setTabHidden(c.closest('.lchip').dataset.id, !c.checked)) c.checked = true;
});

/* Dragging: tabs within their own bar, sections within their own panel. Pointer events, not HTML5
   drag and drop: an app that embeds the page may take over the webview's drag handling (Tauri does
   on Windows), and then dragstart/drop never reach the page. */
let dragEl = null, press = null, justDragged = false;
function dropSpot(el, x, y, across) {
  const r = el.getBoundingClientRect();
  return across ? x > r.left + r.width / 2 : y > r.top + r.height / 2;
}
function clearMarks() { $$('.drop-before, .drop-after').forEach(x => x.classList.remove('drop-before', 'drop-after')); }
function targetAt(x, y) {
  if (!dragEl) return null;
  const under = document.elementFromPoint(x, y);
  const t = dragEl.matches('details') ? under?.closest?.('.rack > details.sec') : under?.closest?.('.tab, .subtab');
  return t && t !== dragEl && t.parentElement === dragEl.parentElement ? t : null;
}
function endDrag() {
  clearMarks();
  dragEl?.classList.remove('dragging');
  document.body.classList.remove('dragging-layout');
  dragEl = null; press = null;
}
document.addEventListener('pointerdown', e => {
  if (!editing || e.button !== 0) return;
  const tab = e.target.closest?.('.tabs .tab, .subtabs .subtab');
  const sum = e.target.closest?.('.rack > details.sec > summary');
  const el = tab || (sum && sum.parentElement);
  if (el) press = { el, x: e.clientX, y: e.clientY, id: e.pointerId };
});
document.addEventListener('pointermove', e => {
  if (!press || e.pointerId !== press.id) return;
  if (!dragEl) {
    // a press that moves a few pixels is a drag; less is a click
    if (Math.hypot(e.clientX - press.x, e.clientY - press.y) < 5) return;
    dragEl = press.el;
    dragEl.classList.add('dragging');
    document.body.classList.add('dragging-layout');
  }
  e.preventDefault();
  clearMarks();
  const t = targetAt(e.clientX, e.clientY);
  if (t) t.classList.add(dropSpot(t, e.clientX, e.clientY, !dragEl.matches('details')) ? 'drop-after' : 'drop-before');
});
document.addEventListener('pointerup', e => {
  if (!press || e.pointerId !== press.id) return;
  const t = dragEl && targetAt(e.clientX, e.clientY);
  if (dragEl) justDragged = true;
  if (t) {
    const after = dropSpot(t, e.clientX, e.clientY, !dragEl.matches('details'));
    if (dragEl.matches('details')) {
      const tool = dragEl.closest('.tool').id.slice(5), order = L.secs[tool].order;
      const from = order.indexOf(dragEl.dataset.sec);
      let to = order.indexOf(t.dataset.sec) + (after ? 1 : 0);
      if (from < to) to--;
      moveSec(tool, dragEl.dataset.sec, to);
    } else {
      const id = dragEl.id.slice(4), list = listOf(id);
      const from = list.indexOf(id);
      let to = list.indexOf(t.id.slice(4)) + (after ? 1 : 0);
      if (from < to) to--;
      if (move(list, id, to)) changed();
    }
  }
  endDrag();
  setTimeout(() => { justDragged = false; }, 0);
});
document.addEventListener('pointercancel', endDrag);
// the click that ends a drag doesn't also open a section or switch tabs
document.addEventListener('click', e => {
  if (justDragged) { e.preventDefault(); e.stopPropagation(); justDragged = false; }
}, true);

/* ---------- saved layouts ---------- */
function fillSaved(selected) {
  const sel = $('#layout-saved'), keep = selected ?? sel.value;
  const names = Object.keys(saved).sort((a, b) => a.localeCompare(b));
  const esc = s => s.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;');
  sel.innerHTML = `<option value="">${names.length ? 'Choose a saved layout…' : 'No saved layouts yet'}</option>` + names.map(n => `<option value="${esc(n)}">${esc(n)}</option>`).join('');
  sel.value = names.includes(keep) ? keep : '';
  $('#layout-del').disabled = !sel.value;
}
$('#layout-saved').addEventListener('change', e => {
  const name = e.target.value;
  $('#layout-del').disabled = !name;
  if (!name || !saved[name]) return;
  L = normalize(clone(saved[name]));
  changed();
  toast(`Switched to the “${name}” layout.`);
});
$('#layout-save').addEventListener('click', () => {
  const inp = $('#layout-name'), name = inp.value.trim().slice(0, 40);
  if (!name) { toast('Type a name for the layout first.'); inp.focus(); return; }
  saved[name] = clone(L);
  store.set('layouts', saved);
  inp.value = ''; fillSaved(name);
  toast(`Saved the layout as “${name}”.`);
});
$('#layout-del').addEventListener('click', () => {
  const name = $('#layout-saved').value;
  if (!name) return;
  delete saved[name];
  store.set('layouts', saved);
  fillSaved('');
  toast(`Deleted the “${name}” layout.`);
});
$('#layout-reset').addEventListener('click', () => {
  // Back to the built-in order with every tab and section showing, and sections open or closed as they ship
  L = normalize({});
  for (const d of $$('.rack > details.sec')) d.open = d.dataset.openDefault === '1';
  changed();
  toast('Back to the standard layout. Your saved layouts are still there.');
});
$('#layout-export').addEventListener('click', () => {
  downloadBlob(new Blob([JSON.stringify({ app: 'clip-studio', kind: 'layouts', version: 1, current: L, layouts: saved }, null, 2)], { type: 'application/json' }), 'clip-studio-layouts.json');
  toast('Exported your layouts.');
});
$('#layout-import').addEventListener('click', () => $('#layout-file').click());
$('#layout-file').addEventListener('change', async e => {
  const f = e.target.files[0]; e.target.value = '';
  if (!f) return;
  try {
    const data = JSON.parse(await f.text());
    if (data?.kind !== 'layouts') throw new Error();
    let n = 0;
    for (const [name, v] of Object.entries(data.layouts || {})) if (v && typeof v === 'object') { saved[name.slice(0, 40)] = v; n++; }
    store.set('layouts', saved);
    if (data.current) { L = normalize(data.current); changed(); }
    fillSaved();
    toast(`Imported ${n} saved layout${n === 1 ? '' : 's'}${data.current ? ' and switched to the exported one' : ''}.`);
  } catch (err) { toast('That file isn’t a Clip Studio layouts file. Use one made with Export here.'); }
});

/* ---------- start-up ---------- */
function init(opts) {
  onChange = opts.onChange || onChange;
  for (const id of [...opts.tabs, ...Object.values(opts.groups).flat()]) labels[id] = $('#tab-' + id).textContent.trim();
  DEFAULT = { tabs: opts.tabs.slice(), groups: clone(opts.groups), secs: {} };
  for (const rack of $$('.tool > .rack')) {
    const tool = rack.parentElement.id.slice(5), keys = [];
    for (const d of $$(':scope > details.sec', rack)) {
      const sum = $(':scope > summary', d), k = secKey(sum.textContent.trim());
      d.dataset.sec = k; d.dataset.openDefault = d.open ? '1' : '0'; keys.push(k);
      // Move, hide, and show controls, visible only while editing; they must not open or close the section
      const tools = document.createElement('span');
      tools.className = 'sec-tools';
      tools.innerHTML = '<button type="button" class="sec-btn" data-act="up" aria-label="Move this section up">↑</button>'
        + '<button type="button" class="sec-btn" data-act="down" aria-label="Move this section down">↓</button>'
        + '<button type="button" class="sec-btn sec-eye" data-act="eye" aria-pressed="true" aria-label="Hide this section"></button>';
      sum.appendChild(tools);
      tools.addEventListener('click', e => {
        e.preventDefault(); e.stopPropagation();
        const b = e.target.closest('button');
        if (!b) return;
        const s = L.secs[tool], i = s.order.indexOf(k);
        if (b.dataset.act === 'eye') setSecHidden(tool, k, !s.hidden.includes(k));
        else moveSec(tool, k, i + (b.dataset.act === 'up' ? -1 : 1));
        $(`#tool-${tool} details.sec[data-sec="${k}"] .sec-btn[data-act="${b.dataset.act}"]`)?.focus();
      });
      // Remember open and closed sections (reads the state itself, so it also holds for the toggle the parser fires)
      d.addEventListener('toggle', () => {
        if (!L) return;
        const s = L.secs[tool];
        if (d.open === (d.dataset.openDefault === '1')) delete s.open[k]; else s.open[k] = d.open;
        save();
      });
    }
    DEFAULT.secs[tool] = keys;
  }
  L = normalize(store.get('layout', null));
  apply();
  $('#layout-edit').addEventListener('click', () => setEditing(!editing));
  $('#layout-done').addEventListener('click', () => setEditing(false));   // shown on narrow screens, where the top bar scrolls
  document.addEventListener('keydown', e => { if (e.key === 'Escape' && editing && !document.querySelector('dialog[open]')) setEditing(false); });
}

window.CS.layout = {
  init, visibleTabs: () => visibleTabs(), visibleItems: g => visibleItems(g), visibleTools, isGroup,
  get editing() { return editing; },
  debug: { get layout() { return L; }, get savedLayouts() { return saved; } },
};
})();
