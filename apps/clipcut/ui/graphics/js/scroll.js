/* Clip Studio · Scroll Zoom tool
   Classic script (not an ES module) so index.html still works when opened straight from disk.
   Shared helpers live on window.CS; each tool registers itself in CS.tools.

   A document (PDF pages or images) is laid out as one column of pages, each PAGE_W units wide.
   The clip scrolls quickly down to each highlight, zooms in, and draws it while the rest of the page dims.
   Pages become bitmaps on demand at a few fixed widths (LEVELS) and are cached within a pixel budget:
   the preview draws whatever is ready and repaints as pages arrive; the export waits for every page a frame needs.
   pdf.js loads only when a PDF is opened. Its worker script is loaded as a plain script so pdf.js runs on the
   main thread: browsers refuse cross-origin workers, and a page opened from file:// has no origin to share. */
(() => {
'use strict';
const {
  $, $$, clamp, lerp, EASE, fmtSec, slug, clone, store, toast, roundRect, loadImageFile,
  FRAMES, dims, resync, saveSoon, bindRange, bindColor, bindCheck, bindSeg, bindText,
  buildOutputPanel, buildPresetPanel, makePlayer, fitViewer, drawGuides, runRender, renderBoth, saveStill,
} = window.CS;

const PDFJS = window.CS.lib('pdfjs', 'https://cdn.jsdelivr.net/npm/pdfjs-dist@3.11.174/');
// Text recognition for pages with no text layer (scans, photos, PDFs whose letters were turned into shapes)
const TESS = {
  lib: window.CS.lib('tesseract-lib', 'https://cdn.jsdelivr.net/npm/tesseract.js@5.1.1/dist/tesseract.min.js'),
  worker: window.CS.lib('tesseract-worker', 'https://cdn.jsdelivr.net/npm/tesseract.js@5.1.1/dist/worker.min.js'),
  core: window.CS.lib('tesseract-core', 'https://cdn.jsdelivr.net/npm/tesseract.js-core@5.1.1'),
  lang: window.CS.lib('tesseract-lang', 'https://cdn.jsdelivr.net/npm/@tesseract.js-data/eng@1.0.0/4.0.0_best_int'),
};
const PAGE_W = 1000, PAGE_GAP = 36, MAX_PAGES = 400;
const LEVELS = [500, 1000, 2000, 3200];   // page bitmap widths in pixels
const BUDGET = 70e6;                      // pixels of page bitmaps kept in memory
const UI_STACK = 'Barlow, "Helvetica Neue", Arial, sans-serif';
const SERIF = 'Georgia, "Times New Roman", serif';

const SZ_DEFAULT = {
  style: 'highlight', color: '#FFD60A', opacity: 0.85, thick: 6, dim: 0.45,
  bg: '#2B2A28', shadow: true, counter: true, label: '',
  pageSize: 92, zoom: 2.6, startHold: 0.6, scrollDur: 1.8, scrollEase: 'whip', blur: 0.6,
  zoomDur: 0.9, drawDur: 0.8, hold: 1.6, endOut: false,
  out: { aspect: '16:9', quality: '1080', fps: 30, format: 'mp4' },
};
const SZ_PRESET_KEYS = ['style', 'color', 'opacity', 'thick', 'dim', 'bg', 'shadow', 'counter', 'pageSize', 'zoom', 'startHold', 'scrollDur', 'scrollEase', 'blur', 'zoomDur', 'drawDur', 'hold', 'endOut', 'out'];
const SZ_BUILTINS = {
  'Bill or law (yellow highlighter)': { style: 'highlight', color: '#FFD60A', opacity: 0.85, dim: 0.45, bg: '#2B2A28', shadow: true, counter: true, pageSize: 92, zoom: 2.6, scrollDur: 1.8, scrollEase: 'whip', blur: 0.6, zoomDur: 0.9, drawDur: 0.8, hold: 1.6, endOut: false },
  'Court record (red box)': { style: 'box', color: '#FF3B30', thick: 6, dim: 0.55, bg: '#1C1B1A', shadow: true, counter: true, pageSize: 90, zoom: 2.4, scrollDur: 2.2, scrollEase: 'whip', blur: 0.7, zoomDur: 1.0, drawDur: 0.9, hold: 2, endOut: false },
  'Quick flip to a line (9:16)': { style: 'underline', color: '#FF3B30', thick: 7, dim: 0.5, bg: '#111111', shadow: false, counter: true, pageSize: 96, zoom: 3, scrollDur: 1.4, scrollEase: 'whip', blur: 0.8, zoomDur: 0.7, drawDur: 0.6, hold: 1.4, endOut: false, out: { aspect: '9:16' } },
  'Calm read-through': { style: 'highlight', color: '#FFD60A', opacity: 0.7, dim: 0.3, bg: '#E6E1D7', shadow: true, counter: false, pageSize: 88, zoom: 2, scrollDur: 3.2, scrollEase: 'smooth', blur: 0.2, zoomDur: 1.4, drawDur: 1.2, hold: 2.2, endOut: true },
};
const SCROLL_EASE = {
  whip: t => (t < 0.5 ? 16 * t ** 5 : 1 - Math.pow(-2 * t + 2, 5) / 2),  // slow start, very fast middle, soft landing
  smooth: EASE.smooth,
  gentle: t => (1 - Math.cos(Math.PI * t)) / 2,
};

const sz = Object.assign(clone(SZ_DEFAULT), store.get('sz', {}));
sz.out = Object.assign({}, SZ_DEFAULT.out, sz.out || {});
if (!FRAMES[sz.out.aspect]) sz.out.aspect = '16:9';
if (!SCROLL_EASE[sz.scrollEase]) sz.scrollEase = 'whip';

/* Document: pages stacked top to bottom. Highlights ("marks") are lists of boxes in page units:
   { p: page index, x, y, w, h }, with x and w in 0..PAGE_W. Neither is stored between sessions. */
const doc = { name: '', kind: 'sample', pages: [], pdf: null };
let marks = [];
let docGen = 0;            // bumps on every new document so stale renders are dropped
let szTL = null, editing = false, editCam = null;

const toolSz = $('#tool-sz');
const szCanvas = $('#sz-canvas'), szGuides = $('#sz-guides'), szViewer = $('#sz-viewer'), szMonitor = $('#sz-monitor');

function layoutPages() {
  let y = 0;
  doc.pages.forEach(pg => { pg.top = y; y += pg.h + PAGE_GAP; });
}
const docHeight = () => (doc.pages.length ? doc.pages[doc.pages.length - 1].top + doc.pages[doc.pages.length - 1].h : PAGE_W);

/* ---------- page bitmaps: render on demand, cache within a pixel budget ---------- */
const cache = new Map();    // `${page}:${level}` → { bmp, px, used }
const pending = new Map();  // same key → promise of a bitmap
let cachePx = 0, useTick = 0, renderChain = Promise.resolve();
const levelFor = px => LEVELS.find(l => l >= px * 0.9) || LEVELS[LEVELS.length - 1];
function evict() {
  if (cachePx <= BUDGET) return;
  const old = [...cache.entries()].filter(([, v]) => v.used < useTick).sort((a, b) => a[1].used - b[1].used);
  for (const [k, v] of old) {
    if (cachePx <= BUDGET) break;
    cache.delete(k); cachePx -= v.px;
  }
}
function requestPage(i, level) {
  const key = `${i}:${level}`;
  const hit = cache.get(key);
  if (hit) { hit.used = useTick; return Promise.resolve(hit.bmp); }
  if (pending.has(key)) return pending.get(key);
  const gen = docGen;
  const job = renderChain.then(async () => {
    if (gen !== docGen) return null;
    const bmp = await doc.pages[i].render(level);
    if (gen !== docGen) return null;
    const px = bmp instanceof HTMLImageElement ? 0 : bmp.width * bmp.height;  // images are already in memory
    cache.set(key, { bmp, px, used: useTick }); cachePx += px;
    evict();
    return bmp;
  });
  const done = job.finally(() => pending.delete(key));
  renderChain = done.catch(() => {});
  pending.set(key, done);
  return done;
}
/** Best bitmap available right now: the requested level, else the nearest one cached. Records misses for the preview. */
function pageImage(i, level, misses) {
  const exact = cache.get(`${i}:${level}`);
  if (exact) { exact.used = useTick; return exact.bmp; }
  misses?.add(`${i}:${level}`);
  const order = [...LEVELS.filter(l => l > level), ...LEVELS.filter(l => l < level).reverse()];  // sharper first
  for (const l of order) { const c = cache.get(`${i}:${l}`); if (c) { c.used = useTick; return c.bmp; } }
  return null;
}
async function ensure(needs) {
  useTick++;
  needs.forEach(n => { const c = cache.get(`${n.i}:${n.level}`); if (c) c.used = useTick; });
  await Promise.all(needs.map(n => requestPage(n.i, n.level)));
}
function resetCache() {
  docGen++; cache.clear(); pending.clear(); cachePx = 0; renderChain = Promise.resolve();
  textCache.clear(); ocrCache.clear(); findState = null;
}

/* ---------- sample document (clearly marked) so the first view shows the effect ---------- */
const SAMPLE_SECTIONS = [
  ['SEC. 1. SHORT TITLE.', 'This Act may be cited as the “Clean Pipes Act of 2026”.'],
  ['SEC. 2. FINDINGS.', 'Congress finds the following: (1) An estimated nine million homes still receive drinking water through lead service lines. (2) No safe level of lead exposure has been identified for children. (3) Replacing a service line costs between four thousand and twelve thousand dollars, a cost many small water systems cannot carry alone. (4) Partial replacement can raise lead levels for months, so full replacement is the only lasting remedy.'],
  ['SEC. 3. DEFINITIONS.', 'In this Act: (1) The term “Administrator” means the Administrator of the Environmental Protection Agency. (2) The term “community water system” has the meaning given the term in section 1401 of the Safe Drinking Water Act. (3) The term “lead service line” means a pipe, made in whole or in part of lead, that connects a water main to a building inlet. (4) The term “disadvantaged community” means a community that meets the affordability criteria set by the State.'],
  ['SEC. 4. INVENTORY OF SERVICE LINES.', 'Not later than one year after the date of enactment of this Act, each community water system shall complete and publish an inventory of every service line in its distribution system, including the material of each line on both the public and private side of the property line. The Administrator shall provide technical assistance to systems serving fewer than ten thousand people.'],
  ['SEC. 5. GRANT PROGRAM.', 'The Administrator shall award grants to States to carry out lead service line replacement. A State receiving a grant shall use not less than forty-nine percent of the funds for projects in disadvantaged communities, and may use not more than four percent for administration. Grants may cover the full cost of replacement on private property, including restoration of landscaping and pavement.'],
  ['SEC. 6. REPLACEMENT REQUIREMENT.', 'Except as provided in subsection (b), each community water system shall replace every lead service line within ten years after the date of enactment of this Act. A system may request one extension of not more than five years if it shows that replacement within ten years is not feasible, and the Administrator shall publish each request and the reasons for granting or denying it.'],
  ['SEC. 7. PUBLIC NOTICE.', 'A community water system shall notify each customer served by a lead service line not later than thirty days after completing its inventory, and again not less than forty-five days before replacement work begins. Notices shall be written in plain language and provided in each language spoken by more than five percent of the people served.'],
  ['SEC. 8. REPORTS TO CONGRESS.', 'Not later than two years after the date of enactment of this Act, and every year thereafter, the Administrator shall submit to Congress a report on the number of lead service lines replaced in each State, the amount of grant funds spent, and the number of systems that requested an extension.'],
  ['SEC. 9. AUTHORIZATION OF APPROPRIATIONS.', 'There is authorized to be appropriated to carry out this Act $15,000,000,000 for each of fiscal years 2027 through 2031, to remain available until expended.'],
  ['SEC. 10. EFFECTIVE DATE.', 'This Act takes effect ninety days after the date of enactment.'],
];
function makeSampleDoc() {
  const PH = 1294, M = 100, BODY = 22, LH = 34, HEAD = 22;
  const mc = document.createElement('canvas').getContext('2d');
  const pages = [[]];
  let y = M + 40;
  const add = (op) => pages[pages.length - 1].push(op);
  const room = need => { if (y + need > PH - M) { pages.push([]); y = M + 40; } };
  add({ str: 'THE CLEAN PIPES ACT', x: PAGE_W / 2, y: M + 70, size: 40, weight: 700, align: 'center' });
  add({ str: 'A sample bill for trying out Scroll Zoom. Open a PDF or images to use your own document.', x: PAGE_W / 2, y: M + 116, size: 19, weight: 400, align: 'center', color: '#666' });
  y = M + 190;
  for (const [head, body] of SAMPLE_SECTIONS) {
    room(LH * 3);
    add({ str: head, x: M, y, size: HEAD, weight: 700 }); y += LH * 1.3;
    mc.font = `400 ${BODY}px ${SERIF}`;
    const words = body.split(' ');
    let line = '';
    const flush = () => { room(LH); add({ str: line, x: M, y, size: BODY, weight: 400 }); y += LH; line = ''; };
    for (const w of words) {
      const next = line ? line + ' ' + w : w;
      if (mc.measureText(next).width > PAGE_W - 2 * M && line) { flush(); line = w; } else line = next;
    }
    if (line) flush();
    y += LH * 0.9;
  }
  return pages.map((ops, pi) => {
    const items = ops.map(op => {
      mc.font = `${op.weight} ${op.size}px ${SERIF}`;
      const w = mc.measureText(op.str).width;
      return { str: op.str, x: op.align === 'center' ? op.x - w / 2 : op.x, y: op.y, w, h: op.size };
    });
    return {
      w: PAGE_W, h: PH, top: 0,
      async render(level) {
        const k = level / PAGE_W;
        const c = document.createElement('canvas'); c.width = level; c.height = Math.round(PH * k);
        const g = c.getContext('2d');
        g.fillStyle = '#ffffff'; g.fillRect(0, 0, c.width, c.height);
        g.scale(k, k);
        g.fillStyle = '#9a9590'; g.font = `600 15px ${UI_STACK}`; g.textAlign = 'right';
        g.fillText(`SAMPLE DOCUMENT · PAGE ${pi + 1}`, PAGE_W - M, M - 30);
        for (const op of ops) {
          g.font = `${op.weight} ${op.size}px ${SERIF}`; g.fillStyle = op.color || '#161514';
          g.textAlign = op.align || 'left'; g.fillText(op.str, op.x, op.y);
        }
        return c;
      },
      async text() { return items; },
    };
  });
}

/* ---------- opening files ---------- */
let pdfjsReady = null;
function loadScript(src) {
  return new Promise((res, rej) => {
    const s = document.createElement('script');
    s.src = src; s.onload = res; s.onerror = () => { s.remove(); rej(new Error(src)); };
    document.head.appendChild(s);
  });
}
function loadPdfjs() {
  if (window.pdfjsLib && window.pdfjsWorker) return Promise.resolve(window.pdfjsLib);
  if (!pdfjsReady) {
    pdfjsReady = Promise.all([loadScript(PDFJS + 'build/pdf.min.js'), loadScript(PDFJS + 'build/pdf.worker.min.js')])
      .then(() => window.pdfjsLib)
      .catch(() => { pdfjsReady = null; throw new Error('The PDF reader didn’t load. Check your internet connection and reload, or save the pages as images and open those.'); });
  }
  return pdfjsReady;
}
function pdfPages(lib, pdf, sizes) {
  return sizes.map((vp1, i) => ({
    w: PAGE_W, h: PAGE_W * vp1.height / vp1.width, top: 0,
    async render(level) {
      const page = await pdf.getPage(i + 1);
      const vp = page.getViewport({ scale: level / vp1.width });
      const c = document.createElement('canvas'); c.width = Math.round(vp.width); c.height = Math.round(vp.height);
      const g = c.getContext('2d');
      g.fillStyle = '#ffffff'; g.fillRect(0, 0, c.width, c.height);
      await page.render({ canvasContext: g, viewport: vp }).promise;
      return c;
    },
    async text() {
      const page = await pdf.getPage(i + 1);
      const vp = page.getViewport({ scale: 1 });
      const k = PAGE_W / vp.width;
      const tc = await page.getTextContent();
      return tc.items.filter(it => it.str && it.str.trim()).map(it => {
        const tx = lib.Util.transform(vp.transform, it.transform);
        const fh = Math.hypot(tx[2], tx[3]);
        return { str: it.str, x: tx[4] * k, y: tx[5] * k, w: it.width * vp.scale * k, h: fh * k };
      });
    },
  }));
}
function imagePage(img) {
  const iw = img.naturalWidth, ih = img.naturalHeight;
  return { w: PAGE_W, h: PAGE_W * ih / iw, top: 0, isImage: true, async render() { return img; }, async text() { return []; } };
}
async function openFiles(files) {
  const list = [...files];
  const pdf = list.find(f => f.type === 'application/pdf' || /\.pdf$/i.test(f.name));
  const imgs = list.filter(f => /^image\//.test(f.type));
  const st = $('#sz-doc-status');
  st.className = 'status';
  try {
    if (pdf) {
      if (list.length > 1) toast('Opened the PDF only. Open images on their own to use them as pages.');
      st.textContent = 'Opening the PDF…';
      const lib = await loadPdfjs();
      let task;
      try {
        task = await lib.getDocument({ data: new Uint8Array(await pdf.arrayBuffer()), standardFontDataUrl: PDFJS + 'standard_fonts/', cMapUrl: PDFJS + 'cmaps/', cMapPacked: true, isEvalSupported: false, verbosity: 0 }).promise;
      } catch (e) {
        throw new Error(e?.name === 'PasswordException' ? 'This PDF is password protected. Open it in a PDF viewer, save an unlocked copy, and open that.' : `“${pdf.name}” couldn’t be read as a PDF. Try saving it again from a PDF viewer.`);
      }
      const n = Math.min(task.numPages, MAX_PAGES);
      const sizes = [];
      for (let i = 1; i <= n; i++) {
        if (i % 10 === 0) st.textContent = `Reading page ${i} of ${n}…`;
        const vp = (await task.getPage(i)).getViewport({ scale: 1 });
        sizes.push({ width: vp.width, height: vp.height });
      }
      setDoc({ name: pdf.name, kind: 'pdf', pdf: task, pages: pdfPages(lib, task, sizes) });
      st.textContent = task.numPages > n ? `Showing the first ${n} of ${task.numPages} pages.` : '';
    } else if (imgs.length) {
      st.textContent = 'Opening images…';
      const pages = [];
      for (const f of imgs) pages.push(imagePage(await loadImageFile(f)));
      setDoc({ name: imgs.length === 1 ? imgs[0].name : `${imgs.length} images`, kind: 'images', pdf: null, pages });
      st.textContent = '';
    } else {
      throw new Error('Choose a PDF, or one or more images (PNG or JPG).');
    }
  } catch (e) { st.textContent = e.message || String(e); st.className = 'status err'; }
}
function setDoc(d) {
  if (doc.pdf && doc.pdf !== d.pdf) doc.pdf.destroy().catch(() => {});
  resetCache();
  Object.assign(doc, d);
  layoutPages();
  marks = [];
  szRefresh();
  setEdit(true, startCam());
  szMarksUI(); szDocUI();
  szPlayer.seek(0);
}

/* ---------- finding text ---------- */
const textCache = new Map(), ocrCache = new Map();
let findState = null;   // { q, next } so pressing Add again adds the next match
const normChar = c => c.normalize('NFKC').toLowerCase().replace(/[‘’]/g, "'").replace(/[“”]/g, '"').replace(/[‐-―−]/g, '-');
function pageText(i) {
  if (!textCache.has(i)) textCache.set(i, doc.pages[i].text().catch(() => []));
  return textCache.get(i);
}
/* Text recognition (Tesseract.js, loaded on first use). One worker is kept for the session; ocrProgress receives
   its progress so the status line can show it. Words come back as text items in page units, with the line's
   baseline and height, so matchesOnPage boxes them exactly like PDF text. */
let ocrWorkerP = null, ocrProgress = null;
function ocrWorker() {
  if (!ocrWorkerP) {
    ocrWorkerP = (window.Tesseract ? Promise.resolve() : loadScript(TESS.lib))
      .then(() => window.Tesseract.createWorker('eng', 1, {
        workerPath: TESS.worker, corePath: TESS.core, langPath: TESS.lang,
        logger: m => ocrProgress?.(m),
      }))
      .catch(e => { ocrWorkerP = null; throw new Error(`The text reader didn’t load (${e?.message || e}). Check your internet connection, or use Draw boxes instead.`); });
  }
  return ocrWorkerP;
}
async function ocrPage(i) {
  if (ocrCache.has(i)) return ocrCache.get(i);
  const gen = docGen, pg = doc.pages[i];
  // A sharp bitmap of the page: PDFs at 2000 px wide; images at their own size, kept between 1600 and 3000 px
  const src = await requestPage(i, pg.isImage ? LEVELS[LEVELS.length - 1] : 2000);
  const nat = src.naturalWidth || src.width, w = clamp(nat, 1600, 3000);
  const c = document.createElement('canvas'); c.width = w; c.height = Math.round(w * pg.h / pg.w);
  const g = c.getContext('2d'); g.fillStyle = '#fff'; g.fillRect(0, 0, c.width, c.height); g.drawImage(src, 0, 0, c.width, c.height);
  const worker = await ocrWorker();
  const { data } = await worker.recognize(c, {}, { text: false, blocks: true, hocr: false, tsv: false });
  if (gen !== docGen) return [];
  const k = PAGE_W / c.width, items = [];
  for (const b of data.blocks || []) for (const p of b.paragraphs || []) for (const line of p.lines || []) {
    const bl = line.baseline || { x0: line.bbox.x0, y0: line.bbox.y1, x1: line.bbox.x1, y1: line.bbox.y1 };
    const baseAt = x => (bl.x1 === bl.x0 ? bl.y0 : bl.y0 + (bl.y1 - bl.y0) * (x - bl.x0) / (bl.x1 - bl.x0));
    for (const wd of line.words || []) {
      const t = (wd.text || '').trim();
      if (!t || wd.confidence < 25) continue;
      const base = baseAt((wd.bbox.x0 + wd.bbox.x1) / 2);
      const h = Math.max(4, (base - line.bbox.y0) / 0.82);   // so matchesOnPage's top (baseline − 0.82h) is the line top
      items.push({ str: t, x: wd.bbox.x0 * k, y: base * k, w: (wd.bbox.x1 - wd.bbox.x0) * k, h: h * k });
    }
  }
  ocrCache.set(i, items);
  return items;
}
/** Boxes (page units) for every match of q on page i. Whitespace is ignored on both sides, so line breaks and
    PDF word spacing don't matter. */
function matchesOnPage(items, q) {
  const chars = [];
  items.forEach((it, ii) => {
    for (let ci = 0; ci < it.str.length; ci++) {
      if (/\s/.test(it.str[ci])) continue;
      for (const ch of normChar(it.str[ci])) chars.push({ ch, ii, ci });
    }
  });
  const hay = chars.map(c => c.ch).join('');
  const needle = [...q].filter(c => !/\s/.test(c)).map(normChar).join('');
  const out = [];
  if (!needle) return out;
  for (let at = hay.indexOf(needle); at >= 0; at = hay.indexOf(needle, at + needle.length)) {
    const run = chars.slice(at, at + needle.length);
    const boxes = [];
    for (const c of run) {
      const it = items[c.ii], n = Math.max(1, it.str.length);
      const x0 = it.x + it.w * (c.ci / n), x1 = it.x + it.w * ((c.ci + 1) / n);
      const top = it.y - it.h * 0.82, bottom = it.y + it.h * 0.24;
      const last = boxes[boxes.length - 1];
      if (last && Math.abs(last.top - top) < it.h * 0.4 && x0 - last.x1 < it.h * 1.5) { last.x1 = Math.max(last.x1, x1); last.x0 = Math.min(last.x0, x0); last.bottom = Math.max(last.bottom, bottom); last.top = Math.min(last.top, top); }
      else boxes.push({ x0, x1, top, bottom });
    }
    out.push(boxes.map(b => ({ x: b.x0, y: b.top, w: b.x1 - b.x0, h: b.bottom - b.top })));
  }
  return out;
}
/* Only one search runs at a time. While one runs, Add reads Stop; a stop, a new document, or a new search makes
   the running one give up at its next page. */
let findRun = 0, searching = false;
async function findAndAdd(raw) {
  if (searching) { findRun++; return false; }
  const btn = $('#sz-find-add');
  searching = true; btn.textContent = 'Stop';
  try { return await findOnce(raw); } finally { searching = false; btn.textContent = 'Add'; ocrProgress = null; }
}
async function findOnce(raw) {
  const q = (raw || '').trim();
  const st = $('#sz-find-status');
  if (!q) { st.textContent = 'Type the words to highlight, exactly as they appear.'; st.className = 'status err'; return false; }
  const nth = findState && findState.q === q ? findState.next : 0;
  st.textContent = 'Searching…'; st.className = 'status';
  let seen = 0, read = 0;
  const run = ++findRun, gen = docGen;
  const stopped = () => {
    if (run === findRun && gen === docGen) return false;
    if (gen === docGen) { st.textContent = 'Search stopped.'; st.className = 'status'; }
    return true;
  };
  for (let i = 0; i < doc.pages.length; i++) {
    if (doc.pages.length > 20 && i % 10 === 0) st.textContent = `Searching page ${i + 1} of ${doc.pages.length}…`;
    let items = await pageText(i);
    if (stopped()) return false;
    if (!items.length && doc.kind !== 'sample') {
      // No text layer on this page: read it with text recognition (once per page; results are kept)
      const first = !ocrCache.size && !ocrWorkerP;
      const label = `Page ${i + 1} of ${doc.pages.length} has no text layer, so it’s being read${first ? ' (the first time downloads the text reader, about 6 MB)' : ''}…`;
      st.textContent = label;
      ocrProgress = m => { if (m.status === 'recognizing text') st.textContent = `Reading page ${i + 1} of ${doc.pages.length}… ${Math.round(m.progress * 100)}%`; };
      try { items = await ocrPage(i); read++; }
      catch (e) { if (!stopped()) { st.textContent = e.message; st.className = 'status err'; } return false; }
      ocrProgress = null;
      if (stopped()) return false;
    }
    const found = matchesOnPage(items, q);
    if (seen + found.length > nth) {
      const boxes = found[nth - seen].map(b => ({ p: i, ...b }));
      marks.push({ boxes, text: q, color: null });
      findState = { q, next: nth + 1 };
      st.textContent = `Added the match on page ${i + 1}. Press Add again for the next one.`; st.className = 'status ok';
      szMarksUI();
      if (editing) setEdit(false); else szRefresh();
      szPlayer.seek(szTL.items[szTL.items.length - 1].t1);  // show the new highlight
      return true;
    }
    seen += found.length;
  }
  const recognized = read > 0 || ocrCache.size > 0;
  st.textContent = nth ? `No more matches for “${q}”.`
    : recognized ? `No match for “${q}”. These pages were read by text recognition, which can miss or misread words. Try a shorter phrase, or use Draw boxes.`
    : `No match for “${q}”. Check the spelling, or use Draw boxes instead.`;
  st.className = 'status err';
  findState = null;
  return false;
}

/* ---------- timeline and camera ---------- */
/* Camera: view center (cx, cy) in document units and scale s (frame pixels per unit),
   applied as translate(W/2, H/2) scale(s) translate(-cx, -cy). */
function markBox(mk) {
  let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
  for (const b of mk.boxes) {
    const top = doc.pages[b.p].top;
    x0 = Math.min(x0, b.x); x1 = Math.max(x1, b.x + b.w); y0 = Math.min(y0, top + b.y); y1 = Math.max(y1, top + b.y + b.h);
  }
  return { x0, y0, x1, y1, w: x1 - x0, h: y1 - y0, cx: (x0 + x1) / 2, cy: (y0 + y1) / 2 };
}
function szTimeline(W, H) {
  const s0 = Math.min(W, H) * sz.pageSize / 100 / PAGE_W;
  const docH = docHeight(), m = PAGE_GAP;
  const fit = c => {
    const hw = W / 2 / c.s, hh = H / 2 / c.s;
    return {
      s: c.s,
      cx: PAGE_W + 2 * m <= 2 * hw ? PAGE_W / 2 : clamp(c.cx, hw - m, PAGE_W + m - hw),
      cy: docH + 2 * m <= 2 * hh ? docH / 2 : clamp(c.cy, hh - m, docH + m - hh),
    };
  };
  const start = fit({ s: s0, cx: PAGE_W / 2, cy: -Infinity });
  const segs = [];
  let t = sz.startHold, cur = start;
  const push = (type, dur, to, fn) => {
    if (dur > 0.01) { segs.push({ type, t0: t, t1: t + dur, a: cur, b: to, ease: fn }); t += dur; }
    cur = to;
  };
  const scrollTo = to => {
    const dist = Math.abs(to.cy - cur.cy);
    push('scroll', dist < 1 ? 0 : sz.scrollDur * clamp(dist / (H / s0 * 2.5), 0.35, 1), to, SCROLL_EASE[sz.scrollEase]);
  };
  const items = marks.map((mk, k) => {
    const bb = markBox(mk);
    const sFit = Math.min(W * 0.86 / Math.max(1, bb.w), H * 0.5 / Math.max(1, bb.h));
    const s = Math.max(s0, Math.min(s0 * sz.zoom, sFit));
    return { mk, k, bb, focus: fit({ s, cx: bb.cx, cy: bb.cy }), over: fit({ s: s0, cx: PAGE_W / 2, cy: bb.cy }) };
  });
  items.forEach((it, k) => {
    const prev = items[k - 1];
    const near = prev && Math.abs(it.bb.cy - prev.bb.cy) < H / it.focus.s * 0.6 && Math.abs(Math.log(it.focus.s / prev.focus.s)) < 0.5;
    if (near) push('pan', 0.9, it.focus, EASE.smooth);
    else {
      if (prev) push('zoom', sz.zoomDur * 0.8, prev.over, EASE.smooth);
      scrollTo(it.over);
      push('zoom', sz.zoomDur, it.focus, EASE.smooth);
    }
    it.t0 = t + 0.05; it.t1 = it.t0 + Math.max(0.05, sz.drawDur);
    t = it.t1 + sz.hold; it.tOut = t;
  });
  if (!items.length) { scrollTo(fit({ s: s0, cx: PAGE_W / 2, cy: Infinity })); t += sz.hold; }
  else if (sz.endOut) { push('zoom', sz.zoomDur, items[items.length - 1].over, EASE.smooth); t += 0.6; }
  return { W, H, s0, start, segs, items, total: Math.max(1, t), docH };
}
function mixCam(a, b, e) {
  const s = Math.exp(lerp(Math.log(a.s), Math.log(b.s), e));
  // While zooming, move the center in step with 1/s so the target grows in place instead of sliding
  const u = Math.abs(a.s - b.s) > 1e-6 ? (1 / a.s - 1 / s) / (1 / a.s - 1 / b.s) : e;
  return { s, cx: lerp(a.cx, b.cx, u), cy: lerp(a.cy, b.cy, u) };
}
function camAt(TL, t) {
  let cam = TL.start;
  for (const g of TL.segs) {
    if (t < g.t0) break;
    if (t >= g.t1) { cam = g.b; continue; }
    return mixCam(g.a, g.b, g.ease(clamp((t - g.t0) / (g.t1 - g.t0), 0, 1)));
  }
  return cam;
}
function visiblePages(cam, W, H) {
  const hh = H / 2 / cam.s, y0 = cam.cy - hh, y1 = cam.cy + hh, out = [];
  doc.pages.forEach((pg, i) => { if (pg.top < y1 && pg.top + pg.h > y0) out.push(i); });
  return out;
}
/** Cameras to draw for one frame (several when motion blur is on) and the page bitmaps they need. */
function framePlan(t, TL, W, H, q, fps, cam = null) {
  let cams;
  if (cam) cams = [cam];
  else {
    const c0 = camAt(TL, t);
    cams = [c0];
    // Blur only while scrolling: on zooms and pans it reads as doubled text rather than speed
    const inScroll = TL.segs.some(g => g.type === 'scroll' && g.t0 < t && g.t1 > t - sz.blur / fps);
    const shutter = inScroll ? sz.blur / fps : 0;
    if (shutter > 0) {
      const c1 = camAt(TL, Math.max(0, t - shutter));
      const px = (Math.abs(c1.cy - c0.cy) + Math.abs(c1.cx - c0.cx)) * c0.s + Math.abs(Math.log(c1.s / c0.s)) * Math.min(W, H);
      const n = clamp(Math.ceil(px / 6), 1, 16);
      for (let j = 1; j < n; j++) cams.push(camAt(TL, Math.max(0, t - shutter * j / n)));
    }
  }
  const needs = new Map();
  for (const c of cams) {
    const level = levelFor(PAGE_W * c.s * q);
    for (const i of visiblePages(c, W, H)) needs.set(`${i}:${level}`, { i, level });
  }
  return { cams, needs: [...needs.values()] };
}

/* ---------- drawing ---------- */
const lift = (t, t0, t1, r = 0.25) => clamp((t - t0) / r, 0, 1) * clamp((t1 - t) / r, 0, 1);
function drawPlaceholderPage(ctx, pg) {
  ctx.fillStyle = '#e9e6e1';
  for (let y = pg.top + 140; y < pg.top + pg.h - 100; y += 34) ctx.fillRect(100, y, (y / 34) % 7 < 1 ? 520 : 800, 12);
}
function drawWorld(ctx, W, H, cam, t, TL, q, { edit, misses }) {
  ctx.save();
  ctx.fillStyle = sz.bg; ctx.fillRect(0, 0, W, H);
  ctx.translate(W / 2, H / 2); ctx.scale(cam.s, cam.s); ctx.translate(-cam.cx, -cam.cy);
  const level = levelFor(PAGE_W * cam.s * q);
  for (const i of visiblePages(cam, W, H)) {
    const pg = doc.pages[i];
    ctx.save();
    if (sz.shadow) { ctx.shadowColor = 'rgba(0,0,0,0.45)'; ctx.shadowBlur = Math.min(W, H) * 0.02; ctx.shadowOffsetY = Math.min(W, H) * 0.006; }
    ctx.fillStyle = '#ffffff'; ctx.fillRect(0, pg.top, PAGE_W, pg.h);
    ctx.restore();
    const img = pageImage(i, level, misses);
    if (img) ctx.drawImage(img, 0, pg.top, PAGE_W, pg.h); else drawPlaceholderPage(ctx, pg);
  }
  TL.items.forEach(it => {
    const p = edit ? 1 : t <= it.t0 ? 0 : t >= it.t1 ? 1 : EASE.smooth((t - it.t0) / (it.t1 - it.t0));
    if (p > 0) drawMark(ctx, it, p);
  });
  ctx.restore();
}
function drawMark(ctx, it, p) {
  const color = it.mk.color || sz.color;
  const boxes = it.mk.boxes.map(b => ({ x: b.x, y: doc.pages[b.p].top + b.y, w: b.w, h: b.h }));
  const th = sz.thick / it.focus.s;   // thickness is set in frame pixels at the zoomed-in view
  ctx.save();
  if (sz.style === 'highlight') {
    ctx.globalCompositeOperation = 'multiply';
    ctx.globalAlpha *= sz.opacity; ctx.fillStyle = color;
    const pads = boxes.map(b => b.h * 0.12);
    let remain = p * boxes.reduce((s, b, k) => s + b.w + pads[k] * 2, 0);
    boxes.forEach((b, k) => {
      if (remain <= 0) return;
      const w = Math.min(b.w + pads[k] * 2, remain); remain -= w;
      roundRect(ctx, b.x - pads[k], b.y - b.h * 0.06, w, b.h * 1.12, b.h * 0.14); ctx.fill();
    });
  } else if (sz.style === 'underline') {
    ctx.strokeStyle = color; ctx.lineWidth = th; ctx.lineCap = 'round';
    let remain = p * boxes.reduce((s, b) => s + b.w, 0);
    boxes.forEach(b => {
      if (remain <= 0) return;
      const w = Math.min(b.w, remain); remain -= w;
      const y = b.y + b.h + th * 0.6;
      ctx.beginPath(); ctx.moveTo(b.x, y); ctx.lineTo(b.x + w, y); ctx.stroke();
    });
  } else if (sz.style === 'box') {
    const bb = it.bb, pad = Math.max(th * 1.5, Math.min(bb.h, 40) * 0.3);
    const x = bb.x0 - pad, y = bb.y0 - pad, w = bb.w + pad * 2, h = bb.h + pad * 2;
    const perim = 2 * (w + h);
    ctx.strokeStyle = color; ctx.lineWidth = th; ctx.lineJoin = 'round';
    ctx.setLineDash([perim * p, perim]);
    roundRect(ctx, x, y, w, h, Math.min(pad, h / 2)); ctx.stroke();
  }
  ctx.restore();
}
function drawDim(ctx, W, H, cam, t, TL) {
  if (sz.dim <= 0) return;
  for (const it of TL.items) {
    const fadeIn = clamp((t - it.t0) / Math.max(0.05, it.t1 - it.t0), 0, 1);
    const fadeOut = 1 - clamp((t - it.tOut) / 0.4, 0, 1);
    const a = sz.dim * EASE.smooth(fadeIn) * fadeOut;
    if (a <= 0.001) continue;
    ctx.save();
    ctx.translate(W / 2, H / 2); ctx.scale(cam.s, cam.s); ctx.translate(-cam.cx, -cam.cy);
    ctx.beginPath();
    ctx.rect(cam.cx - W / cam.s, cam.cy - H / cam.s, 2 * W / cam.s, 2 * H / cam.s);
    for (const b of it.mk.boxes) {
      const pad = b.h * 0.3, y = doc.pages[b.p].top + b.y;
      if (ctx.roundRect) ctx.roundRect(b.x - pad, y - pad, b.w + pad * 2, b.h + pad * 2, pad);
      else ctx.rect(b.x - pad, y - pad, b.w + pad * 2, b.h + pad * 2);
    }
    ctx.fillStyle = `rgba(0,0,0,${a})`; ctx.fill('evenodd');
    ctx.restore();
  }
}
function pill(ctx, text, x, y, size, alignRight) {
  ctx.font = `600 ${size}px ${UI_STACK}`;
  const w = ctx.measureText(text).width + size * 1.3, h = size * 1.9;
  const px = alignRight ? x - w : x;
  ctx.fillStyle = 'rgba(12,11,10,0.78)'; roundRect(ctx, px, y, w, h, h / 2); ctx.fill();
  ctx.fillStyle = '#ffffff'; ctx.textBaseline = 'middle'; ctx.textAlign = 'left';
  ctx.fillText(text, px + size * 0.65, y + h / 2 + size * 0.04);
}
const layer = document.createElement('canvas');
function szDraw(ctx, W, H, t, TL, plan, { q = 1, edit = false, misses = null } = {}) {
  const minSide = Math.min(W, H);
  ctx.save();
  if (plan.cams.length === 1) drawWorld(ctx, W, H, plan.cams[0], t, TL, q, { edit, misses });
  else {
    // Motion blur: draw each camera position opaque on its own layer, then average the layers (equal weights)
    const cw = ctx.canvas.width, ch = ctx.canvas.height, m = ctx.getTransform();
    if (layer.width !== cw || layer.height !== ch) { layer.width = cw; layer.height = ch; }
    const lc = layer.getContext('2d');
    plan.cams.forEach((cam, j) => {
      lc.setTransform(m); drawWorld(lc, W, H, cam, t, TL, q, { edit, misses });
      ctx.save(); ctx.setTransform(1, 0, 0, 1, 0, 0); ctx.globalAlpha = 1 / (j + 1); ctx.drawImage(layer, 0, 0); ctx.restore();
    });
  }
  const cam = plan.cams[0];
  if (!edit) drawDim(ctx, W, H, cam, t, TL);
  else drawEditMarks(ctx, W, H, cam);
  const m = minSide * 0.045, size = minSide * 0.024;
  if (sz.label.trim()) pill(ctx, sz.label.trim(), m, m, size, false);
  if (sz.counter && !edit && doc.pages.length > 1) {
    const a = Math.max(0, ...TL.segs.filter(g => g.type === 'scroll').map(g => lift(t, g.t0 - 0.1, g.t1 + 0.15)));
    if (a > 0) {
      const y = cam.cy;
      let pi = doc.pages.findIndex(pg => y < pg.top + pg.h + PAGE_GAP / 2);
      if (pi < 0) pi = doc.pages.length - 1;
      ctx.globalAlpha = a;
      pill(ctx, `Page ${pi + 1} of ${doc.pages.length}`, W - m, m, size, true);
      ctx.globalAlpha = 1;
    }
  }
  ctx.restore();
}
function drawEditMarks(ctx, W, H, cam) {
  ctx.save();
  ctx.translate(W / 2, H / 2); ctx.scale(cam.s, cam.s); ctx.translate(-cam.cx, -cam.cy);
  const lw = 2 / cam.s;
  ctx.lineWidth = lw;
  marks.forEach((mk, i) => mk.boxes.forEach((b, bi) => {
    const y = doc.pages[b.p].top + b.y;
    ctx.setLineDash([lw * 4, lw * 3]); ctx.strokeStyle = 'rgba(244,196,67,0.95)';
    ctx.strokeRect(b.x, y, b.w, b.h);
    if (bi === 0) {
      const pw = 26 / cam.s;
      ctx.setLineDash([]); ctx.fillStyle = '#f4c443';
      roundRect(ctx, b.x - pw * 0.2, y - pw * 1.05, pw, pw * 0.9, pw * 0.2); ctx.fill();
      ctx.fillStyle = '#1d1600'; ctx.font = `700 ${pw * 0.62}px ${UI_STACK}`; ctx.textAlign = 'center'; ctx.textBaseline = 'alphabetic';
      ctx.fillText(String(i + 1), b.x + pw * 0.3, y - pw * 0.36);
    }
  }));
  if (drag) {
    ctx.setLineDash([]); ctx.strokeStyle = '#ffffff'; ctx.fillStyle = 'rgba(244,196,67,0.18)';
    const x = Math.min(drag.x0, drag.x1), y = Math.min(drag.y0, drag.y1), w = Math.abs(drag.x1 - drag.x0), h = Math.abs(drag.y1 - drag.y0);
    ctx.fillRect(x, y, w, h); ctx.strokeRect(x, y, w, h);
  }
  ctx.restore();
}

/* ---------- preview ---------- */
let repaintQueued = false;
function requestRepaint() {
  if (repaintQueued) return;
  repaintQueued = true;
  requestAnimationFrame(() => { repaintQueued = false; if (!szPlayer.playing) szPaint(szPlayer.t); });
}
function szPaint(t) {
  if (!szTL) return;
  const { W, H } = dims(sz.out);
  if (szCanvas.width !== W || szCanvas.height !== H) { szCanvas.width = W; szCanvas.height = H; }
  const ctx = szCanvas.getContext('2d');
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  useTick++;
  const misses = new Set();
  const plan = framePlan(t, szTL, W, H, 1, sz.out.fps, editing ? editCam : null);
  szDraw(ctx, W, H, t, szTL, plan, { edit: editing, misses });
  for (const key of misses) {
    const [i, level] = key.split(':').map(Number);
    if (doc.pages[i]) requestPage(i, level).then(requestRepaint, () => {});
  }
  const g = szGuides.getContext('2d');
  const dpr = window.devicePixelRatio || 1;
  const gw = Math.round(szViewer.clientWidth * dpr), gh = Math.round(szViewer.clientHeight * dpr);
  if (szGuides.width !== gw || szGuides.height !== gh) { szGuides.width = gw; szGuides.height = gh; }
  g.clearRect(0, 0, gw, gh);
  if (szPlayer.guides) drawGuides(g, gw, gh, sz.out.aspect, gw / W * 2);
}
function szRenderTrack() {
  const tr = $('#sz-track'), TL = szTL, T = TL.total;
  const blk = (from, len, c, label, dim) => `<div class="blk${dim ? ' dim' : ''}" style="left:${from / T * 100}%;width:${Math.max(0.3, len / T * 100)}%;--c:${c}">${label ? `<span>${label}</span>` : ''}</div>`;
  let html = '';
  TL.segs.forEach(g => { html += blk(g.t0, g.t1 - g.t0, '#8a857c', g.type === 'scroll' ? 'Scroll' : '', g.type !== 'scroll'); });
  TL.items.forEach((it, k) => { html += blk(it.t0, it.t1 - it.t0, it.mk.color || sz.color, String(k + 1)); });
  tr.innerHTML = html + '<div class="head"></div>';
}
function startCam() { const { W, H } = dims(sz.out); return szTimeline(W, H).start; }
function szRefresh() {
  const { W, H, OW, OH } = dims(sz.out);
  szTL = szTimeline(W, H);
  toolSz.dataset.style = sz.style;
  const n = doc.pages.length;
  $('#sz-fmt').textContent = `${OW} × ${OH} · ${n} page${n === 1 ? '' : 's'} · ${marks.length} highlight${marks.length === 1 ? '' : 's'} · ${sz.out.fps} fps`;
  $('#sz-note').textContent = editing ? 'Drawing boxes: drag over the words. Scroll to move; Ctrl/⌘ + scroll to zoom.' : '';
  $('#sz-note').classList.toggle('live', editing);
  $('#sz-outnote').textContent = 'Pages are drawn sharp at every zoom, so long PDFs can take a while to render. Keep this tab in front.';
  szRenderTrack();
  if (szPlayer.t > szTL.total) szPlayer.seek(szTL.total); else { szPaint(szPlayer.t); szPlayer.sync(); }
  saveSoon('sz', sz);
}
function szFit() { fitViewer(szMonitor, szViewer, sz.out.aspect); szPaint(szPlayer.t); }

/* ---------- document and highlight lists ---------- */
function szDocUI() {
  $('#sz-docname').textContent = doc.kind === 'sample'
    ? 'Sample document. Open a PDF or images to use your own.'
    : `${doc.name} · ${doc.pages.length} page${doc.pages.length === 1 ? '' : 's'}`;
  $('#sz-sample-tag').hidden = doc.kind !== 'sample';
  $('#sz-edit').setAttribute('aria-pressed', String(editing));
  $('#sz-goto').max = String(doc.pages.length);
}
function szMarksUI() {
  const list = $('#sz-marks');
  list.innerHTML = '';
  marks.forEach((mk, i) => {
    const li = document.createElement('li');
    const pages = [...new Set(mk.boxes.map(b => b.p + 1))].join(', ');
    li.innerHTML = `<input type="color" aria-label="Color for highlight ${i + 1}"><button class="t" type="button"><span class="n">${i + 1}</span></button><button class="x" type="button" aria-label="Remove highlight ${i + 1}">×</button>`;
    li.querySelector('.t').append(`p. ${pages} · ${mk.text ? `“${mk.text}”` : mk.boxes.length === 1 ? '1 box' : mk.boxes.length + ' boxes'}`);
    li.querySelector('.t').title = 'Show this highlight';
    const c = li.querySelector('input'); c.value = (mk.color || sz.color).toLowerCase();
    c.addEventListener('input', () => { mk.color = c.value; szRefresh(); });
    li.querySelector('.t').addEventListener('click', () => {
      const it = szTL.items[i]; if (!it) return;
      if (editing) { editCam = { ...it.focus }; szPaint(szPlayer.t); } else szPlayer.seek(it.t1);
    });
    li.querySelector('.x').addEventListener('click', () => { marks.splice(i, 1); findState = null; szMarksUI(); szRefresh(); });
    list.appendChild(li);
  });
  $('#sz-marks-clear').disabled = !marks.length;
}
function setEdit(on, cam = null) {
  editing = on;
  if (on) { szPlayer.pause(); editCam = cam || editCam || (szTL ? camAt(szTL, szPlayer.t) : startCam()); }
  szViewer.classList.toggle('editing', on);
  szDocUI(); szRefresh();
}

/* ---------- preview interactions: pick files, draw boxes, scroll and zoom while editing ---------- */
const frameXY = e => {
  const r = szCanvas.getBoundingClientRect(), { W, H } = dims(sz.out);
  return { fx: (e.clientX - r.left) / r.width * W, fy: (e.clientY - r.top) / r.height * H, W, H, k: W / r.width };
};
const toDoc = (fx, fy, W, H, cam) => ({ x: (fx - W / 2) / cam.s + cam.cx, y: (fy - H / 2) / cam.s + cam.cy });
let drag = null;
szCanvas.addEventListener('pointerdown', e => {
  if (!editing || !editCam) return;
  e.preventDefault();
  try { szCanvas.setPointerCapture(e.pointerId); } catch (_) { /* ignore */ }
  const { fx, fy, W, H } = frameXY(e), p = toDoc(fx, fy, W, H, editCam);
  drag = { x0: p.x, y0: p.y, x1: p.x, y1: p.y };
});
szCanvas.addEventListener('pointermove', e => {
  if (!drag) return;
  const { fx, fy, W, H } = frameXY(e), p = toDoc(fx, fy, W, H, editCam);
  drag.x1 = p.x; drag.y1 = p.y; szPaint(szPlayer.t);
});
szCanvas.addEventListener('pointerup', () => {
  const d = drag; drag = null;
  if (!d) return;
  const y0 = Math.min(d.y0, d.y1), y1 = Math.max(d.y0, d.y1);
  const pi = doc.pages.findIndex(pg => (y0 + y1) / 2 >= pg.top && (y0 + y1) / 2 <= pg.top + pg.h);
  const pg = doc.pages[pi];
  const x = clamp(Math.min(d.x0, d.x1), 0, PAGE_W), x2 = clamp(Math.max(d.x0, d.x1), 0, PAGE_W);
  const minUnits = 6 / editCam.s;
  if (!pg || x2 - x < minUnits || y1 - y0 < minUnits) { szPaint(szPlayer.t); return; }
  const box = { p: pi, x, y: clamp(y0 - pg.top, 0, pg.h), w: x2 - x, h: Math.min(y1, pg.top + pg.h) - Math.max(y0, pg.top) };
  if ($('#sz-join').checked && marks.length) marks[marks.length - 1].boxes.push(box);
  else marks.push({ boxes: [box], text: '', color: null });
  szMarksUI(); szRefresh();
});
szCanvas.addEventListener('pointercancel', () => { drag = null; szPaint(szPlayer.t); });
function zoomEdit(factor, fx, fy) {
  const { W, H } = dims(sz.out);
  fx ??= W / 2; fy ??= H / 2;
  const before = toDoc(fx, fy, W, H, editCam);
  const s0 = szTL.s0;
  editCam.s = clamp(editCam.s * factor, s0 * 0.4, s0 * 10);
  editCam.cx = before.x - (fx - W / 2) / editCam.s; editCam.cy = before.y - (fy - H / 2) / editCam.s;
  szPaint(szPlayer.t);
}
szCanvas.addEventListener('wheel', e => {
  if (!editing || !editCam) return;
  e.preventDefault();
  const { fx, fy, k } = frameXY(e);
  if (e.ctrlKey || e.metaKey) zoomEdit(Math.exp(-e.deltaY * 0.01), fx, fy);
  else {
    editCam.cx += e.deltaX * k / editCam.s; editCam.cy += e.deltaY * k / editCam.s;
    editCam.cy = clamp(editCam.cy, -PAGE_W, docHeight() + PAGE_W);
    editCam.cx = clamp(editCam.cx, -PAGE_W / 2, PAGE_W * 1.5);
    szPaint(szPlayer.t);
  }
}, { passive: false });
$('#sz-edit').addEventListener('click', () => setEdit(!editing));
$('#sz-fitw').addEventListener('click', () => { if (!editing) setEdit(true); editCam = { s: szTL.s0, cx: PAGE_W / 2, cy: editCam.cy }; szPaint(szPlayer.t); });
$('#sz-zin').addEventListener('click', () => { if (!editing) setEdit(true); zoomEdit(1.25); });
$('#sz-zout').addEventListener('click', () => { if (!editing) setEdit(true); zoomEdit(0.8); });
$('#sz-goto').addEventListener('change', e => {
  const pg = doc.pages[clamp(Math.round(+e.target.value) - 1, 0, doc.pages.length - 1)];
  if (!pg) return;
  if (!editing) setEdit(true);
  const { H } = dims(sz.out);
  editCam = { ...editCam, cy: pg.top + H / 2 / editCam.s - PAGE_GAP };
  szPaint(szPlayer.t);
});
$('#sz-marks-clear').addEventListener('click', () => { marks = []; findState = null; szMarksUI(); szRefresh(); });
$('#sz-find-add').addEventListener('click', () => findAndAdd($('#sz-find').value));
$('#sz-find').addEventListener('keydown', e => { if (e.key === 'Enter') { e.preventDefault(); findAndAdd($('#sz-find').value); } });
$('#sz-find').addEventListener('input', () => { findState = null; });
$('#sz-pick').addEventListener('click', () => $('#sz-file').click());
$('#sz-file').addEventListener('change', e => { const f = [...e.target.files]; e.target.value = ''; if (f.length) openFiles(f); });
for (const el of [$('#sz-drop'), szViewer]) {
  el.addEventListener('dragover', e => { if ([...e.dataTransfer.items].some(i => i.kind === 'file')) { e.preventDefault(); el.classList.add(el === szViewer ? 'dropping' : 'over'); } });
  el.addEventListener('dragleave', () => el.classList.remove('dropping', 'over'));
  el.addEventListener('drop', e => { e.preventDefault(); el.classList.remove('dropping', 'over'); if (e.dataTransfer.files.length) openFiles(e.dataTransfer.files); });
}
document.addEventListener('paste', e => {
  if (toolSz.hidden || e.target?.closest?.('input, textarea')) return;
  const item = [...(e.clipboardData?.items || [])].find(it => it.type.startsWith('image/'));
  if (item) { e.preventDefault(); openFiles([item.getAsFile()]); }
});

/* ---------- controls ---------- */
const szChanged = key => {
  if (key === 'out.aspect') { szFit(); if (editing) editCam = startCam(); }
  szRefresh();
};
bindSeg('sz-style', sz, 'style', szChanged);
bindColor('sz-color', sz, 'color', key => { szMarksUI(); szChanged(key); });
bindRange('sz-opacity', sz, 'opacity', v => Math.round(v * 100) + '%', szChanged);
bindRange('sz-thick', sz, 'thick', v => v + 'px', szChanged);
bindRange('sz-dim', sz, 'dim', v => Math.round(v * 100) + '%', szChanged);
bindColor('sz-bg', sz, 'bg', szChanged);
bindCheck('sz-shadow', sz, 'shadow', szChanged);
bindCheck('sz-counter', sz, 'counter', szChanged);
bindText('sz-label', sz, 'label', szChanged);
bindRange('sz-pagesize', sz, 'pageSize', v => v + '%', szChanged);
bindRange('sz-zoom', sz, 'zoom', v => (+v).toFixed(1) + '×', szChanged);
bindRange('sz-starthold', sz, 'startHold', fmtSec, szChanged);
bindRange('sz-scroll', sz, 'scrollDur', fmtSec, szChanged);
bindSeg('sz-sease', sz, 'scrollEase', szChanged);
bindRange('sz-blur', sz, 'blur', v => Math.round(v * 100) + '%', szChanged);
bindRange('sz-zoomdur', sz, 'zoomDur', fmtSec, szChanged);
bindRange('sz-drawdur', sz, 'drawDur', fmtSec, szChanged);
bindRange('sz-hold', sz, 'hold', fmtSec, szChanged);
bindCheck('sz-endout', sz, 'endOut', szChanged);

const szPlayer = makePlayer('sz', {
  total: () => (szTL ? szTL.total : 1), fps: () => sz.out.fps, render: szPaint, track: $('#sz-track'),
  beforePlay: () => { if (editing) setEdit(false); return true; },
});
// Scrubbing or clicking the timeline leaves box drawing so you see the animation at that moment
$('#sz-p-scrub').addEventListener('input', () => { if (editing) setEdit(false); });
$('#sz-track').addEventListener('pointerdown', () => { if (editing) setEdit(false); });

/* ---------- export ---------- */
async function szRenderOne() {
  szPlayer.pause();
  if (editing) setEdit(false);
  const { q, W, H, OW, OH } = dims(sz.out);
  const TL = szTimeline(W, H), fps = sz.out.fps;
  return runRender({
    title: `Rendering ${marks.length ? `${marks.length} highlight${marks.length === 1 ? '' : 's'}` : 'scroll'}`,
    OW, OH, fps, total: TL.total, format: sz.out.format,
    name: `scroll_${slug((doc.kind === 'sample' ? 'sample' : doc.name).replace(/\.[^.]+$/, '')) || 'document'}_${OW}x${OH}`,
    async frame(ctx, t) {
      const plan = framePlan(t, TL, W, H, q, fps);
      await ensure(plan.needs);
      ctx.setTransform(q, 0, 0, q, 0, 0);
      szDraw(ctx, W, H, t, TL, plan, { q });
    },
  });
}
async function szStill() {
  const { q, W, H, OW, OH } = dims(sz.out);
  const TL = szTimeline(W, H);
  const plan = framePlan(szPlayer.t, TL, W, H, q, sz.out.fps, editing ? editCam : null);
  await ensure(plan.needs);
  const c = document.createElement('canvas'); c.width = OW; c.height = OH;
  const ctx = c.getContext('2d'); ctx.setTransform(q, 0, 0, q, 0, 0);
  szDraw(ctx, W, H, szPlayer.t, TL, plan, { q });
  await saveStill(c, `scroll_frame_${OW}x${OH}.png`);
}
buildOutputPanel('sz', sz.out, szChanged, {
  render: szRenderOne,
  renderBoth: () => renderBoth(sz.out, () => { szFit(); szRefresh(); }, szRenderOne),
  still: szStill,
});
buildPresetPanel('sz', sz, SZ_PRESET_KEYS, SZ_BUILTINS, () => { resync(sz); resync(sz.out); szMarksUI(); szFit(); szRefresh(); });
new ResizeObserver(() => { if (!toolSz.hidden) szFit(); }).observe(szMonitor);

CS.tools.sz = {
  id: 'sz',
  player: szPlayer,
  boot() {
    Object.assign(doc, { name: 'Sample document', kind: 'sample', pdf: null, pages: makeSampleDoc() });
    layoutPages();
    szDocUI(); szRefresh();
    pageText(0); // warm up
    // Sample highlight, found the same way a typed phrase is
    Promise.all(doc.pages.map((_, i) => pageText(i))).then(all => {
      if (doc.kind !== 'sample') return;
      const i = all.findIndex(items => matchesOnPage(items, 'replace every lead service line within ten years').length);
      if (i < 0) return;
      const boxes = matchesOnPage(all[i], 'replace every lead service line within ten years')[0].map(b => ({ p: i, ...b }));
      marks = [{ boxes, text: 'replace every lead service line within ten years', color: null }];
      szMarksUI(); szRefresh();
      szPlayer.seek(szTL.items[0].t1 + 0.3);
    });
  },
  show() { requestAnimationFrame(szFit); },
  hide() { szPlayer.pause(); },
  debug: {
    sz, szPlayer, szDoc: doc,
    get szMarks() { return marks; }, get szTL() { return szTL; }, get szEditing() { return editing; }, get szEditCam() { return editCam; },
  },
};
})();
