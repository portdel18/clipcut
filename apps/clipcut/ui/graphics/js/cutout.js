/* Clip Studio · cut-outs: takes the background off a picture (Thumbnails' photo and inset)
   Classic script (not an ES module) so index.html still works when opened straight from disk.

   ISNet (DIS, general use; Apache-2.0) run by ONNX Runtime Web (MIT): on the graphics card through
   WebGPU when the browser has it, else on the processor (WebAssembly, one thread: about 20 s). The
   model (178 MB) downloads once from rembg's release, is checked against its SHA-256, and is kept in
   IndexedDB; an app holding the page (ClipCut) downloads and keeps it itself (HOST.model). When the
   download fails, the person can choose the model file they downloaded (CS.cutout.useFile).
   CS.cutout(img, progress) → a new <img> the same size, with the background transparent. */
(() => {
'use strict';
const { HOST, lib } = window.CS;
// absolute: ONNX Runtime imports its WebAssembly glue from here, and imports don't take bare paths
const ORT = new URL(lib('onnxruntime-web', 'https://cdn.jsdelivr.net/npm/onnxruntime-web@1.22.0/dist/'), document.baseURI).href;
const MODEL = {
  name: 'isnet-general-use',
  file: 'isnet-general-use.onnx',
  url: 'https://github.com/danielgatis/rembg/releases/download/v0.0.0/isnet-general-use.onnx',
  sha256: '60920e99c45464f2ba57bee2ad08c919a52bbf852739e96947fbb4358c0d964a',
  bytes: 178648008,
  size: 1024,
};
// the longest side a cut-out keeps (bigger photos are cut out at this size)
const MAX_SIDE = 4096;

let session = null, sessionP = null, onGpu = false;

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
async function kept() {
  try {
    const db = await idb();
    return await new Promise(res => {
      const q = db.transaction('models').objectStore('models').get(MODEL.name);
      q.onsuccess = () => res(q.result || null); q.onerror = () => res(null);
    });
  } catch (_) { return null; }
}
async function keep(buf) {
  try {
    const db = await idb();
    await new Promise(res => { const t = db.transaction('models', 'readwrite'); t.objectStore('models').put(buf, MODEL.name); t.oncomplete = res; t.onerror = res; });
  } catch (_) { /* downloads again next visit */ }
}
async function checked(buf, from) {
  if (buf.byteLength !== MODEL.bytes || await hex(buf) !== MODEL.sha256) {
    throw new Error(`The background model ${from} isn't the right file (its size or checksum is wrong). Download ${MODEL.file} again.`);
  }
  return buf;
}
/** The model's bytes: from the app, from IndexedDB, or downloaded (with `progress(done, total)`). */
async function modelBytes(progress) {
  if (HOST && typeof HOST.model === 'function') {
    const from = await HOST.model(MODEL.name, progress);
    // the app's buffer belongs to its own window: ONNX Runtime wants one of this page's
    const buf = new ArrayBuffer(from.byteLength);
    new Uint8Array(buf).set(new Uint8Array(from));
    return checked(buf, 'ClipCut downloaded');
  }
  const have = await kept();
  if (have) { try { return await checked(have, 'kept in this browser'); } catch (_) { /* download it again */ } }
  let r;
  try { r = await fetch(MODEL.url); } catch (_) { r = null; }
  if (!r || !r.ok || !r.body) {
    const e = new Error(`The background model couldn't be downloaded. Download ${MODEL.file} (178 MB) from ${MODEL.url}, then press Choose model file.`);
    e.needsFile = true;
    throw e;
  }
  const reader = r.body.getReader(), parts = [];
  let done = 0;
  for (;;) {
    const { value, done: end } = await reader.read();
    if (end) break;
    parts.push(value); done += value.length;
    progress?.(done, MODEL.bytes);
  }
  const buf = await new Blob(parts).arrayBuffer();
  await checked(buf, 'downloaded');
  await keep(buf);
  return buf;
}

async function makeSession(buf, ep) {
  return window.ort.InferenceSession.create(buf, { executionProviders: [ep], graphOptimizationLevel: 'all' });
}
/** The model, ready to run (once per page). */
function ready(progress) {
  if (session) return Promise.resolve(session);
  if (!sessionP) {
    sessionP = (async () => {
      if (!window.ort) await loadScript(ORT + 'ort.webgpu.min.js');
      const ort = window.ort;
      ort.env.wasm.wasmPaths = ORT;
      ort.env.wasm.numThreads = 1;
      const buf = await modelBytes(progress);
      cutout.buf = buf;
      progress?.(MODEL.bytes, MODEL.bytes, 'start');
      if (navigator.gpu) {
        try { session = await makeSession(buf, 'webgpu'); onGpu = true; return session; } catch (_) { /* the processor, then */ }
      }
      session = await makeSession(buf, 'wasm'); onGpu = false;
      return session;
    })();
    sessionP.catch(() => { sessionP = null; });
  }
  return sessionP;
}

/** The picture `img` (an <img> or canvas) with its background transparent, as a new <img>. */
async function cutout(img, progress) {
  let s = await ready(progress);
  const ort = window.ort, N = MODEL.size;
  const iw = img.naturalWidth || img.width, ih = img.naturalHeight || img.height;
  if (!iw || !ih) throw new Error('That picture has no size.');
  const c = document.createElement('canvas'); c.width = N; c.height = N;
  const g = c.getContext('2d', { willReadFrequently: true });
  g.drawImage(img, 0, 0, N, N);
  const px = g.getImageData(0, 0, N, N).data;
  const x = new Float32Array(3 * N * N);
  for (let i = 0; i < N * N; i++) {
    x[i] = px[i * 4] / 255 - 0.5;
    x[N * N + i] = px[i * 4 + 1] / 255 - 0.5;
    x[2 * N * N + i] = px[i * 4 + 2] / 255 - 0.5;
  }
  const feeds = { [s.inputNames[0]]: new ort.Tensor('float32', x, [1, 3, N, N]) };
  let out;
  try { out = await s.run(feeds); } catch (e) {
    if (!onGpu) throw e;
    // the graphics card failed this one: the processor instead, from now on
    session = await makeSession(cutout.buf || await modelBytes(progress), 'wasm'); onGpu = false;
    s = session;
    out = await s.run(feeds);
  }
  const y = out[s.outputNames[0]].data;
  let mn = Infinity, mx = -Infinity;
  for (let i = 0; i < N * N; i++) { const v = y[i]; if (v < mn) mn = v; if (v > mx) mx = v; }
  const span = mx - mn > 1e-6 ? mx - mn : 1;
  // the matte at the model's size, then scaled to the picture's
  const m = g.createImageData(N, N);
  for (let i = 0; i < N * N; i++) { const a = Math.round(((y[i] - mn) / span) * 255); m.data[i * 4 + 3] = a; }
  g.clearRect(0, 0, N, N);
  g.putImageData(m, 0, 0);
  const k = Math.min(1, MAX_SIDE / Math.max(iw, ih));
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
  return out2;
}
/** Uses a model file the person downloaded themselves (checked, then kept). */
cutout.useFile = async file => {
  const buf = await checked(await file.arrayBuffer(), 'chosen');
  await keep(buf);
  return true;
};
cutout.model = MODEL;
Object.defineProperty(cutout, 'onGpu', { get: () => onGpu });
window.CS.cutout = cutout;
})();
