// The keyboard shortcuts' logic in ui/app.js, run with stand-ins for the page:
// `node apps/clipcut/ui-tests/shortcuts.js apps/clipcut/ui/app.js` (Windows CI runs it).
// Every default key must reach its action, and changing keys must never lose one.
const fs = require('fs');
const src = fs.readFileSync(process.argv[2], 'utf8');
const a = src.indexOf('const nudge = n =>');
const b = src.indexOf("document.addEventListener('keydown'");
const keysPage = src.slice(src.indexOf('/** Takes key `k` off action'), src.indexOf("$('#keys-dlg').addEventListener('keydown'"));
let saved = {};
const stub = `
const $ = () => ({ textContent: '', value: '' });
const document = { querySelectorAll: () => [] };
const renderTools = () => {}, renderShortcuts = () => {}, toast = (e) => { throw new Error(String(e)); };
const has = () => true;
let LOADED = null;
const invoke = (cmd, args) => { if (cmd === 'save_shortcuts') { globalThis.SAVED = args.keys; return Promise.resolve(); } return Promise.resolve(LOADED); };
`;
const code = stub + src.slice(a, b) + keysPage + `
module.exports = { ACTIONS, KEYS, get KEYMAP() { return KEYMAP; }, comboOf, assignKeys, takeKeyOff, useKeyChanges, keyChanges };`;
const m = new module.constructor();
m._compile(code, 'keys.js');
const K = m.exports;
const ok = (c, msg) => { if (!c) { console.error('FAIL:', msg); process.exitCode = 1; } else console.log('ok:', msg); };
const press = (o) => K.KEYMAP.get(K.comboOf({ key: o.key, code: o.code, ctrlKey: !!o.ctrl, shiftKey: !!o.shift, altKey: !!o.alt, metaKey: false }));
const keys = {
  ArrowLeft: { key: 'ArrowLeft', code: 'ArrowLeft' }, ArrowRight: { key: 'ArrowRight', code: 'ArrowRight' },
  ArrowUp: { key: 'ArrowUp', code: 'ArrowUp' }, ArrowDown: { key: 'ArrowDown', code: 'ArrowDown' },
};
// 1. every default key reaches its action
K.useKeyChanges({});
const parse = combo => {
  const m = combo.match(/^((?:(?:Ctrl|Alt|Shift)\+)*)(.+)$/); const parts = m[1].split('+').filter(Boolean); const k = m[2];
  const o = { ctrl: parts.includes('Ctrl'), shift: parts.includes('Shift'), alt: parts.includes('Alt') };
  const codes = { ',': 'Comma', '.': 'Period', '/': 'Slash', '=': 'Equal', '-': 'Minus', 'Num +': 'NumpadAdd', 'Num -': 'NumpadSubtract', Space: 'Space' };
  if (/^[A-Z]$/.test(k)) return { ...o, key: o.shift ? k : k.toLowerCase(), code: 'Key' + k };
  if (/^\d$/.test(k)) return { ...o, key: k, code: 'Digit' + k };
  if (codes[k]) return { ...o, key: k === 'Space' ? ' ' : k, code: codes[k] };
  return { ...o, key: k, code: k };
};
let all = true;
for (const act of K.ACTIONS) for (const k of act.keys) {
  const got = press(parse(k));
  if (!got || got.id !== act.id) { all = false; console.error('FAIL:', k, '->', got && got.id, 'want', act.id); }
}
ok(all, 'every default key reaches its own action');
// 2. ← recorded onto Split moves it; taking it off Split sends it back to Back one frame
K.assignKeys('split', ['ArrowLeft']);
ok(press(keys.ArrowLeft).id === 'split', '← moved to Split');
K.takeKeyOff('split', 'ArrowLeft');
ok(press(keys.ArrowLeft) && press(keys.ArrowLeft).id === 'frame-back', '← is back on Back one frame after taking it off Split');
ok(press(parse('S')).id === 'split', 'S still splits');
// 3. Ctrl+← / Ctrl+→ given to an action and taken off: nothing else changes
K.assignKeys('five-back', ['Ctrl+ArrowLeft']); K.assignKeys('five-on', ['Ctrl+ArrowRight']);
ok(press({ ...keys.ArrowLeft, ctrl: true }).id === 'five-back' && press({ ...keys.ArrowRight, ctrl: true }).id === 'five-on', 'Ctrl+← and Ctrl+→ work once given');
K.takeKeyOff('five-back', 'Ctrl+ArrowLeft'); K.takeKeyOff('five-on', 'Ctrl+ArrowRight');
ok(!press({ ...keys.ArrowLeft, ctrl: true }) && press(keys.ArrowLeft).id === 'frame-back' && press({ ...keys.ArrowLeft, shift: true }).id === 'five-back', 'taken off again: ← and Shift+← unchanged');
// 4. a key taken off its own action stays off (deliberate), and comes back with Default
K.takeKeyOff('frame-on', 'ArrowRight');
ok(!press(keys.ArrowRight), '→ taken off Forward one frame stays off');
K.assignKeys('frame-on', ['ArrowRight'], true);
ok(press(keys.ArrowRight).id === 'frame-on', 'Default gives → back');
// 5. saved and loaded again: the same keys
const saved1 = globalThis.SAVED;
K.useKeyChanges(saved1);
ok(press(keys.ArrowLeft).id === 'frame-back' && press(keys.ArrowRight).id === 'frame-on', 'the same after saving and loading');
// 6. the broken file from before: ← moved to Split and taken off there (Back one frame saved with no keys)
K.useKeyChanges({ 'frame-back': [], split: ['S'] });
ok(press(keys.ArrowLeft) && press(keys.ArrowLeft).id === 'frame-back', 'an old file with ← on nothing gets it back on Back one frame');
ok(globalThis.SAVED._version === 2, 'and is saved as version 2');
K.useKeyChanges({ 'frame-back': [], _version: 2 });
ok(!press(keys.ArrowLeft), 'a version 2 file with ← deliberately off keeps it off');
// 7. moving a key between two actions and back
K.useKeyChanges({});
K.assignKeys('frame-on', ['ArrowUp']);
ok(press(keys.ArrowUp).id === 'frame-on' && !(K.KEYS.get('cut-back') || []).includes('ArrowUp'), '↑ moved to Forward one frame');
K.takeKeyOff('frame-on', 'ArrowUp');
ok(press(keys.ArrowUp).id === 'cut-back' && press(keys.ArrowRight).id === 'frame-on', '↑ back on the previous event, → kept');
