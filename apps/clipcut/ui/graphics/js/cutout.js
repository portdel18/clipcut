/* Clip Studio · cut-outs: takes the background off a picture (Thumbnails' photo and inset)
   Classic script (not an ES module) so index.html still works when opened straight from disk.

   Three models, by quality (`MODELS`): Standard is ISNet (DIS, general use; Apache-2.0, 178 MB);
   Best is BiRefNet general and Best for people BiRefNet portrait (MIT, 973 MB each: finer hair and
   edges, slower). ONNX Runtime Web (MIT) runs them on the graphics card through WebGPU when the
   browser has it, else on the processor (WebAssembly, one thread: about 20 s for Standard, minutes
   for the others). A model downloads once from rembg's release, is checked against its SHA-256, and
   is kept in IndexedDB; an app holding the page (ClipCut) downloads and keeps it itself
   (HOST.model). When the download fails, the person can choose the model file they downloaded
   (CS.cutout.useFile). CS.cutout(img, progress, quality) → a new <img> the same size, with the
   background transparent. */
(() => {
'use strict';
const { HOST, lib } = window.CS;
// absolute: ONNX Runtime imports its WebAssembly glue from here, and imports don't take bare paths
const ORT = new URL(lib('onnxruntime-web', 'https://cdn.jsdelivr.net/npm/onnxruntime-web@1.22.0/dist/'), document.baseURI).href;
const RELEASE = 'https://github.com/danielgatis/rembg/releases/download/v0.0.0/';
/* Each model's input: `mean`/`std` per channel on 0–1 values; `sigmoid` when its output is logits.
   ClipCut pins the same files (`MODELS` in its src/cutout.rs). */
const MODELS = {
  standard: {
    name: 'isnet-general-use', label: 'Standard', file: 'isnet-general-use.onnx',
    sha256: '60920e99c45464f2ba57bee2ad08c919a52bbf852739e96947fbb4358c0d964a', bytes: 178648008,
    size: 1024, mean: [0.5, 0.5, 0.5], std: [1, 1, 1], sigmoid: false,
  },
  best: {
    name: 'birefnet-general', label: 'Best', file: 'BiRefNet-general-epoch_244.onnx',
    sha256: '58f621f00f5d756097615970a88a791584600dcf7c45b18a0a6267535a1ebd3c', bytes: 972666916,
    size: 1024, mean: [0.485, 0.456, 0.406], std: [0.229, 0.224, 0.225], sigmoid: true, gpuOnly: true,
  },
  people: {
    name: 'birefnet-portrait', label: 'Best for people', file: 'BiRefNet-portrait-epoch_150.onnx',
    sha256: '1ba1c8ff5a7bbfadc8d8d13fb11d7be793f91f23d9d466549e37a854f6668f99', bytes: 972666916,
    size: 1024, mean: [0.485, 0.456, 0.406], std: [0.229, 0.224, 0.225], sigmoid: true, gpuOnly: true,
  },
};
for (const m of Object.values(MODELS)) m.url = RELEASE + m.file;
const MODEL = MODELS.standard;
const modelFor = quality => MODELS[quality] || MODELS.standard;
const mb = n => Math.round(n / 1e6);
// the longest side a cut-out keeps (bigger photos are cut out at this size)
const MAX_SIDE = 4096;

// one model in memory at a time (the big ones take a gigabyte or more)
let session = null, sessionP = null, sessionFor = null, onGpu = false;

function loadScript(src) {
  return new Promise((res, rej) => {
    const s = document.createElement('script');
    s.src = src; s.onload = res; s.onerror = () => rej(new Error('ONNX Runtime could not be loaded. Check the internet connection, then try again.'));
    document.head.appendChild(s);
  });
}
async function hex(buf) {
  const d = await crypto.subtle.digest('SHA-256', buf);
  return [...new Uint8Array(d)].map(b => b.toString(16).padStart(2, '0')).join('');
}

/* The model, kept in IndexedDB between visits (where the browser allows it) */
const DB = 'clipstudio-models';
function idb() {
  return new Promise((res, rej) => {
    let r;
    try { r = indexedDB.open(DB, 1); } catch (e) { rej(e); return; }
    r.onupgradeneeded = () => r.result.createObjectStore('models');
    r.onsuccess = () => res(r.result);
    r.onerror = () => rej(r.error);
  });
}
async function kept(model) {
  try {
    const db = await idb();
    return await new Promise(res => {
      const q = db.transaction('models').objectStore('models').get(model.name);
      q.onsuccess = () => res(q.result || null); q.onerror = () => res(null);
    });
  } catch (_) { return null; }
}
async function keep(model, buf) {
  try {
    const db = await idb();
    await new Promise(res => { const t = db.transaction('models', 'readwrite'); t.objectStore('models').put(buf, model.name); t.oncomplete = res; t.onerror = res; });
  } catch (_) { /* downloads again next visit */ }
}
async function checked(model, buf, from) {
  if (buf.byteLength !== model.bytes || await hex(buf) !== model.sha256) {
    throw new Error(`The background model ${from} isn't the right file (its size or checksum is wrong). Download ${model.file} again.`);
  }
  return buf;
}
/** A model's bytes: from the app, from IndexedDB, or downloaded (with `progress(done, total)`). */
async function modelBytes(model, progress) {
  if (HOST && typeof HOST.model === 'function') {
    const from = await HOST.model(model.name, progress);
    // the app's buffer belongs to its own window: ONNX Runtime wants one of this page's
    const buf = new ArrayBuffer(from.byteLength);
    new Uint8Array(buf).set(new Uint8Array(from));
    return checked(model, buf, 'ClipCut downloaded');
  }
  const have = await kept(model);
  if (have) { try { return await checked(model, have, 'kept in this browser'); } catch (_) { /* download it again */ } }
  let r;
  try { r = await fetch(model.url); } catch (_) { r = null; }
  if (!r || !r.ok || !r.body) {
    const e = new Error(`The background model couldn't be downloaded. Download ${model.file} (${mb(model.bytes)} MB) from ${model.url}, then press Choose model file.`);
    e.needsFile = true;
    throw e;
  }
  const reader = r.body.getReader(), parts = [];
  let done = 0;
  for (;;) {
    const { value, done: end } = await reader.read();
    if (end) break;
    parts.push(value); done += value.length;
    progress?.(done, model.bytes);
  }
  const buf = await new Blob(parts).arrayBuffer();
  await checked(model, buf, 'downloaded');
  await keep(model, buf);
  return buf;
}

async function makeSession(buf, ep) {
  return window.ort.InferenceSession.create(buf, { executionProviders: [ep], graphOptimizationLevel: 'all' });
}
/** `model`, ready to run (kept while it is the one asked for). */
function ready(model, progress) {
  if (sessionFor !== model) {
    // another quality: let the one in memory go first
    if (session && session.release) session.release().catch(() => {});
    session = null; sessionP = null; cutout.buf = null; sessionFor = model;
  }
  if (session) return Promise.resolve(session);
  if (!sessionP) {
    sessionP = (async () => {
      if (!window.ort) await loadScript(ORT + 'ort.webgpu.min.js');
      const ort = window.ort;
      ort.env.wasm.wasmPaths = ORT;
      ort.env.wasm.numThreads = 1;
      // before a 973 MB download: the Best models run only on a graphics card the page can use
      if (model.gpuOnly && !(navigator.gpu && await navigator.gpu.requestAdapter().catch(() => null))) {
        throw new Error(`The ${model.label} model needs the graphics card, and this browser can't use it here. Choose Standard instead.`);
      }
      const buf = await modelBytes(model, progress);
      cutout.buf = buf;
      progress?.(model.bytes, model.bytes, 'start');
      if (navigator.gpu) {
        try { session = await makeSession(buf, 'webgpu'); onGpu = true; return session; } catch (_) { /* the processor, then */ }
      }
      // the Best models need the graphics card: on the processor they need more memory than a page has
      if (model.gpuOnly) throw new Error(`The ${model.label} model needs the graphics card, and this browser can't use it here. Choose Standard instead.`);
      session = await makeSession(buf, 'wasm'); onGpu = false;
      return session;
    })();
    sessionP.catch(() => { sessionP = null; });
  }
  return sessionP;
}

/** The picture `img` (an <img> or canvas) with its background transparent, as a new <img> (its PNG
    as `.blob`), by the model of `quality` ('standard', 'best' or 'people'), at most `maxSide` on
    its longest side. */
async function cutout(img, progress, quality = 'standard', { maxSide = MAX_SIDE } = {}) {
  const model = modelFor(quality);
  let s = await ready(model, progress);
  const ort = window.ort, N = model.size;
  const iw = img.naturalWidth || img.width, ih = img.naturalHeight || img.height;
  if (!iw || !ih) throw new Error('That picture has no size.');
  const c = document.createElement('canvas'); c.width = N; c.height = N;
  const g = c.getContext('2d', { willReadFrequently: true });
  g.drawImage(img, 0, 0, N, N);
  const px = g.getImageData(0, 0, N, N).data;
  const x = new Float32Array(3 * N * N);
  const { mean, std } = model;
  for (let i = 0; i < N * N; i++) {
    for (let ch = 0; ch < 3; ch++) x[ch * N * N + i] = (px[i * 4 + ch] / 255 - mean[ch]) / std[ch];
  }
  const feeds = { [s.inputNames[0]]: new ort.Tensor('float32', x, [1, 3, N, N]) };
  let out;
  try { out = await s.run(feeds); } catch (e) {
    if (!onGpu) throw e;
    if (model.gpuOnly) throw new Error(`The graphics card couldn't run the ${model.label} model (${e.message || e}). Choose Standard instead.`);
    // the graphics card failed this one: the processor instead, from now on
    session = await makeSession(cutout.buf || await modelBytes(model, progress), 'wasm'); onGpu = false;
    s = session;
    out = await s.run(feeds);
  }
  const raw = out[s.outputNames[0]].data;
  const y = model.sigmoid ? Float32Array.from(raw.subarray(0, N * N), v => 1 / (1 + Math.exp(-v))) : raw;
  let mn = Infinity, mx = -Infinity;
  for (let i = 0; i < N * N; i++) { const v = y[i]; if (v < mn) mn = v; if (v > mx) mx = v; }
  const span = mx - mn > 1e-6 ? mx - mn : 1;
  // the matte at the model's size, then scaled to the picture's
  const m = g.createImageData(N, N);
  for (let i = 0; i < N * N; i++) { const a = Math.round(((y[i] - mn) / span) * 255); m.data[i * 4 + 3] = a; }
  g.clearRect(0, 0, N, N);
  g.putImageData(m, 0, 0);
  const k = Math.min(1, maxSide / Math.max(iw, ih));
  const W = Math.max(1, Math.round(iw * k)), H = Math.max(1, Math.round(ih * k));
  const o = document.createElement('canvas'); o.width = W; o.height = H;
  const og = o.getContext('2d');
  og.drawImage(c, 0, 0, W, H);
  og.globalCompositeOperation = 'source-in';
  og.drawImage(img, 0, 0, W, H);
  const blob = await new Promise(res => o.toBlob(res, 'image/png'));
  if (!blob) throw new Error('The cut-out could not be made.');
  const out2 = new Image();
  out2.src = URL.createObjectURL(blob);
  await out2.decode();
  out2.blob = blob;
  return out2;
}
/** Uses a model file the person downloaded themselves for `quality` (checked, then kept). */
cutout.useFile = async (file, quality = 'standard') => {
  const model = modelFor(quality);
  const buf = await checked(model, await file.arrayBuffer(), 'chosen');
  await keep(model, buf);
  return true;
};
/* For an app holding the page (ClipCut's Remove background on a picture in its video): a picture
   file → its cut-out as PNG bytes at the picture's own size, so it takes the picture's place. */
const HOST_MAX = 8192;
window.CS.hosted.cutout = async (file, quality = 'standard', progress) => {
  // the app's file is its own window's: read it into one of this page's
  const url = URL.createObjectURL(new Blob([await file.arrayBuffer()], { type: file.type || 'image/png' }));
  try {
    const img = new Image();
    img.src = url;
    try { await img.decode(); } catch (_) { throw new Error('That picture could not be read.'); }
    if (Math.max(img.naturalWidth, img.naturalHeight) > HOST_MAX) throw new Error(`That picture is over ${HOST_MAX} pixels on its longest side: make it smaller, then try again.`);
    const out = await cutout(img, progress, quality, { maxSide: HOST_MAX });
    URL.revokeObjectURL(out.src);
    return new Uint8Array(await out.blob.arrayBuffer());
  } finally { URL.revokeObjectURL(url); }
};
cutout.model = MODEL;
cutout.models = MODELS;
Object.defineProperty(cutout, 'onGpu', { get: () => onGpu });
window.CS.cutout = cutout;
})();
