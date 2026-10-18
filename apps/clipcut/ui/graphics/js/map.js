/* Clip Studio · Map Zoom tool
   Classic script (not an ES module) so index.html still works when opened straight from disk.
   Shared helpers live on window.CS; each tool registers itself in CS.tools. */
(() => {
'use strict';
const {
  $, $$, clamp, lerp, smoothstep, sleep, EASE, ease, easeOutBounce, easeOutBack, mulberry32, pad2, fmtTC, fmtSec, isHex, slug, clone, store, toastTimer, toast, roundRect, loadImageFile, FRAMES, dims, SWATCHES, registry, reg, resync, saveTimers, saveSoon, bindRange, bindColor, bindCheck, bindSeg, bindText, bindSelect, buildOutputPanel, userPresets, presetUI, buildPresetPanel, ICON_PLAY, ICON_PAUSE, ICON_START, makePlayer, fitViewer, drawGuides, pickEncoder, mp4Sink, pngSink, downloadBlob, fmtBytes, render, runRender, renderBoth, saveStill,
} = window.CS;
const bind = { mp: {} };

/* ============================================================
   MAP ZOOM TOOL
   ============================================================ */
const MP_DEFAULT = {
  mode: 'place',
  query: 'Empire State Building, New York',
  target: { lat: 40.748440, lng: -73.985664 }, name: 'Empire State Building', via: 'Sample location', endCenter: null,
  source: 'esri-sat', projection: 'globe', stars: true,
  terrain: false, exaggeration: 1.4, buildings: false, buildingColor: '#E8E2D6',
  startZoom: 2, endZoom: 17, spin: 40, rotate: 0, pitch: 0,
  delay: 0.4, duration: 6, hold: 2, ease: 'smooth',
  marker: 'pin', markerColor: '#FF3B30', label: 'Empire State Building', labelAuto: true, labelStyle: 'pill', labelBg: '#FFFFFF', labelFg: '#111111', coords: true, vignette: true,
  rQueryA: 'New York', rQueryB: 'London',
  rA: { lat: 40.7128, lng: -74.0060 }, rB: { lat: 51.5074, lng: -0.1278 }, rSample: true,
  rLabelA: 'New York', rLabelB: 'London', rTitle: 'New York → London', rLabelAuto: true,
  rPath: 'arc', rBend: 0.22, rRoad: null, rRoadKey: '', rRoadDist: 0,
  // Outline mode: the areas found, each kept with its (simplified) lines, its own label, and optionally its own color
  aKind: 'state', aQuery: '', areas: [], aTitle: '', aTitleAuto: true, aOrder: 'seq', aRadius: 10, aUnit: 'mi',
  aColor: '#FFD60A', aWidth: 4, aFill: 0.16, aDim: 0.35, aDraw: 1.6,
  rStops: [], rPause: 0.6,   // stops between From and To: { q, lat, lng, label }
  rCam: 'fit', rFollowZoom: 11, rDraw: 5, rFlyIn: true,
  lineColor: '#FFD60A', lineWidth: 4, lineGlow: true, lineDash: false, rIcon: 'plane',
  out: { aspect: '16:9', quality: '1080', fps: 30, format: 'mp4' },
};
const MP_PRESET_KEYS = ['source', 'projection', 'stars', 'terrain', 'exaggeration', 'buildings', 'buildingColor', 'startZoom', 'spin', 'rotate', 'pitch', 'delay', 'duration', 'hold', 'ease', 'marker', 'markerColor', 'labelStyle', 'labelBg', 'labelFg', 'coords', 'vignette', 'rPath', 'rBend', 'rCam', 'rFollowZoom', 'rDraw', 'rFlyIn', 'rPause', 'aColor', 'aWidth', 'aFill', 'aDim', 'aDraw', 'aOrder', 'lineColor', 'lineWidth', 'lineGlow', 'lineDash', 'rIcon', 'out'];
const MP_BUILTINS = {
  'Globe to street (satellite)': { source: 'esri-sat', projection: 'globe', stars: true, terrain: false, buildings: false, startZoom: 2, spin: 40, rotate: 0, pitch: 0, duration: 6, delay: 0.4, hold: 2, ease: 'smooth', marker: 'pin', markerColor: '#FF3B30', labelStyle: 'pill', vignette: true },
  '3D city fly-in': { source: 'esri-hybrid', projection: 'globe', stars: true, terrain: true, exaggeration: 1.3, buildings: true, buildingColor: '#E8E2D6', startZoom: 2, spin: 30, rotate: 45, pitch: 60, duration: 7, ease: 'smooth', marker: 'target', markerColor: '#FF3B30', labelStyle: 'lower', vignette: true },
  'Mountains in 3D': { source: 'esri-sat', projection: 'globe', terrain: true, exaggeration: 1.8, buildings: false, startZoom: 2, spin: 25, rotate: -60, pitch: 62, duration: 7, ease: 'smooth', marker: 'pulse', labelStyle: 'lower', vignette: true },
  'Clean streets': { source: 'streets', projection: 'flat', terrain: false, buildings: false, startZoom: 4, spin: 0, rotate: 0, pitch: 0, duration: 4, ease: 'glide', marker: 'pulse', markerColor: '#0A84FF', labelStyle: 'pill', vignette: false },
  'Flight path': { source: 'esri-sat', projection: 'globe', terrain: false, buildings: false, rPath: 'great', rCam: 'fit', rFlyIn: true, startZoom: 1.5, lineColor: '#FFD60A', lineWidth: 4, lineGlow: true, lineDash: false, rIcon: 'plane', rDraw: 5, marker: 'pin', labelStyle: 'pill', pitch: 0, rotate: 0 },
  'Road trip': { source: 'esri-hybrid', projection: 'flat', terrain: false, rPath: 'roads', rCam: 'follow', rFollowZoom: 11, rFlyIn: false, rPause: 0.8, lineColor: '#0A84FF', lineWidth: 5, lineGlow: true, lineDash: false, rIcon: 'dot', rDraw: 7, marker: 'pin', labelStyle: 'pill', pitch: 0, rotate: 0 },
};
const mp = Object.assign(clone(MP_DEFAULT), store.get('mp', {}));
mp.out = Object.assign({}, MP_DEFAULT.out, mp.out || {});
if (!FRAMES[mp.out.aspect]) mp.out.aspect = '16:9';
if (!['place', 'route', 'area'].includes(mp.mode)) mp.mode = 'place';
if (mp.area) { mp.areas = [mp.area]; if (mp.aLabel) mp.aTitle = mp.aLabel; }   // saved by the single-area version
delete mp.area; delete mp.aLabel; delete mp.aLabelAuto;
mp.areas = (Array.isArray(mp.areas) ? mp.areas : []).filter(a => a && Array.isArray(a.rings) && a.rings.length && a.bbox).slice(0, 8);
if (!['state', 'county', 'place', 'cd', 'sldu', 'sldl', 'radius'].includes(mp.aKind)) mp.aKind = 'state';
mp.rStops = (Array.isArray(mp.rStops) ? mp.rStops : []).filter(s => s && typeof s === 'object').slice(0, 8);
let gKey = store.get('gkey', '');

const MAP_SOURCES = {
  'esri-sat': { provider: 'esri', labels: false, credit: 'Imagery © Esri, Maxar, Earthstar Geographics' },
  'esri-hybrid': { provider: 'esri', labels: true, credit: 'Imagery and labels © Esri, Maxar, Earthstar Geographics, HERE, Garmin' },
  'streets': { provider: 'ofm', credit: '© OpenStreetMap contributors · OpenMapTiles · OpenFreeMap' },
  'google-sat': { provider: 'google', mapType: 'satellite', credit: 'Google Maps' },
  'google-hybrid': { provider: 'google', mapType: 'satellite', layerTypes: ['layerRoadmap'], credit: 'Google Maps' },
  'google-road': { provider: 'google', mapType: 'roadmap', credit: 'Google Maps' },
};
const ESRI = 'https://server.arcgisonline.com/ArcGIS/rest/services';
const DEM_TILES = 'https://s3.amazonaws.com/elevation-tiles-prod/terrarium/{z}/{x}/{y}.png';
const OFM_TILEJSON = 'https://tiles.openfreemap.org/planet';
const SKY = { 'sky-color': '#0b1c3a', 'horizon-color': '#6f9de0', 'fog-color': '#a9cbf5', 'sky-horizon-blend': 0.6, 'horizon-fog-blend': 0.6, 'fog-ground-blend': 0.2, 'atmosphere-blend': ['interpolate', ['linear'], ['zoom'], 0, 1, 5, 1, 7, 0] };
let tileCount = store.get('tilecount', { month: '', n: 0 });
const monthKey = new Date().toISOString().slice(0, 7);
if (tileCount.month !== monthKey) tileCount = { month: monthKey, n: 0 };
function countTile(url) {
  if (!url.includes('tile.googleapis.com/v1/2dtiles')) return;
  tileCount.n++;
  if (tileCount.n % 25 === 0) store.set('tilecount', tileCount);
  updateTileBar();
}
function updateTileBar() {
  $('#mp-tiles').textContent = MAP_SOURCES[mp.source].provider === 'google' ? `Google tiles this month in this browser: ${tileCount.n.toLocaleString()}` : '';
}

/* Google Map Tiles sessions */
const gSessions = store.get('gsessions', {});
let googleCopy = '';
function googleErrorText(status, body) {
  let msg = '';
  try { msg = JSON.parse(body).error.message || ''; } catch (e) { msg = body || ''; }
  if (/not valid|API_KEY_INVALID|invalid/i.test(msg)) return 'Google says this API key isn’t valid. Copy it again from Cloud Console → Credentials.';
  if (/not been used|disabled|not authorized|PERMISSION_DENIED|SERVICE_DISABLED/i.test(msg)) return 'Map Tiles API isn’t enabled for this key’s project, or the key is restricted to other APIs. Enable it in APIs & Services → Library.';
  if (/billing/i.test(msg)) return 'Google needs a billing account linked to this project before it serves tiles.';
  if (/referer|referrer/i.test(msg)) return 'The key’s website restriction blocked this page. Set Application restrictions to None, or allow this page’s web address.';
  return `Google returned an error (${status}). ${msg}`.trim();
}
async function googleSession(sourceId) {
  if (!gKey) throw new Error('Add your Google Maps API key under Imagery & 3D, or pick a free imagery source.');
  const tag = gKey.slice(-6);
  const c = gSessions[sourceId];
  if (c && c.tag === tag && +c.expiry * 1000 > Date.now() + 3600e3) return c;
  const def = MAP_SOURCES[sourceId];
  const body = { mapType: def.mapType, language: 'en-US', region: 'US', scale: 'scaleFactor2x', highDpi: true };
  if (def.layerTypes) body.layerTypes = def.layerTypes;
  const url = `https://tile.googleapis.com/v1/createSession?key=${encodeURIComponent(gKey)}`;
  let r;
  try { r = await fetch(url, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) }); }
  catch (e) { throw new Error('Couldn’t reach Google. Check your connection; if you restricted the key to websites, set it to None.'); }
  if (!r.ok) {
    const first = await r.text();
    delete body.scale; delete body.highDpi;
    r = await fetch(url, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) });
    if (!r.ok) throw new Error(googleErrorText(r.status, (await r.text()) || first));
  }
  const j = await r.json();
  const s = { session: j.session, expiry: j.expiry, tileWidth: j.tileWidth || 256, tag };
  gSessions[sourceId] = s; store.set('gsessions', gSessions);
  return s;
}
async function googleCopyright(sourceId) {
  try {
    const s = await googleSession(sourceId);
    const { LW, LH } = mpLogical();
    let lat, lng, z;
    if (mp.mode === 'route' && routeGeo) { const f = routeFit(); lat = f.lat; lng = ((f.lng + 540) % 360) - 180; z = Math.round(f.zoom); }
    else if (mp.mode === 'area' && mp.areas.length) { const u = areasFrame(); lat = u.center.lat; lng = u.center.lng; z = Math.round(clamp(u.zoom, 1, 22)); }
    else { const end = mp.endCenter || mp.target; lat = end.lat; lng = end.lng; z = Math.round(clamp(mp.endZoom, 1, 22)); }
    const lngSpan = Math.min(359, LW * 360 / (256 * Math.pow(2, z)));
    const latSpan = Math.min(170, lngSpan * (LH / LW) * Math.cos(lat * Math.PI / 180));
    const q = new URLSearchParams({ session: s.session, key: gKey, zoom: z, north: clamp(lat + latSpan / 2, -89.9, 89.9), south: clamp(lat - latSpan / 2, -89.9, 89.9), east: clamp(lng + lngSpan / 2, -179.9, 179.9), west: clamp(lng - lngSpan / 2, -179.9, 179.9) });
    const r = await fetch(`https://tile.googleapis.com/tile/v1/viewport?${q}`);
    const j = await r.json();
    return j.copyright || '';
  } catch (e) { return ''; }
}
function mpCreditText() {
  const def = MAP_SOURCES[mp.source];
  const parts = [def.provider === 'google' ? (googleCopy ? `Google Maps · ${googleCopy}` : 'Google Maps') : def.credit];
  if (mp.terrain) parts.push('Elevation: Mapzen Terrain Tiles');
  if (mp.buildings && def.provider !== 'ofm') parts.push('Buildings © OpenStreetMap');
  if (mp.mode === 'route' && routeGeo?.isRoad) parts.push('Route: OSRM, © OpenStreetMap');
  if (mp.mode === 'area') parts.push(...new Set(mp.areas.map(a => a.credit).filter(Boolean)));
  return parts.join(' · ');
}

function mpLogical() { const [w, h] = FRAMES[mp.out.aspect]; return { LW: w / 2, LH: h / 2 }; }

/* Style for a pixel ratio R. Raster tiles are sized so each image pixel lands on one output pixel. */
function buildingLayer(source) {
  return {
    id: 'cs-buildings', type: 'fill-extrusion', source, 'source-layer': 'building', minzoom: 13,
    paint: {
      'fill-extrusion-color': mp.buildingColor,
      'fill-extrusion-height': ['coalesce', ['get', 'render_height'], 8],
      'fill-extrusion-base': ['coalesce', ['get', 'render_min_height'], 0],
      'fill-extrusion-opacity': 0.92,
    },
  };
}
async function mpStyle(sourceId, R) {
  const def = MAP_SOURCES[sourceId];
  const projection = { type: mp.projection === 'globe' ? 'globe' : 'mercator' };
  if (def.provider === 'ofm') return { url: 'https://tiles.openfreemap.org/styles/liberty', projection };
  const sources = {};
  const layers = [{ id: 'base', type: 'background', paint: { 'background-color': '#0b1d2c' } }];
  const add = (id, tiles, tileSize, maxzoom) => {
    sources[id] = { type: 'raster', tiles, tileSize: Math.max(32, Math.round(tileSize)), maxzoom };
    layers.push({ id, type: 'raster', source: id, paint: { 'raster-fade-duration': 0 } });
  };
  if (def.provider === 'esri') {
    add('imagery', [`${ESRI}/World_Imagery/MapServer/tile/{z}/{y}/{x}`], 256 / R, 18);
    if (def.labels) {
      add('roads', [`${ESRI}/Reference/World_Transportation/MapServer/tile/{z}/{y}/{x}`], 256, 18);
      add('places', [`${ESRI}/Reference/World_Boundaries_and_Places/MapServer/tile/{z}/{y}/{x}`], 256, 18);
    }
  } else {
    const s = await googleSession(sourceId);
    add('imagery', [`https://tile.googleapis.com/v1/2dtiles/{z}/{x}/{y}?session=${encodeURIComponent(s.session)}&key=${encodeURIComponent(gKey)}`], s.tileWidth / R, 21);
  }
  const style = { version: 8, projection, sky: SKY, sources, layers };
  if (mp.terrain) {
    sources.dem = { type: 'raster-dem', tiles: [DEM_TILES], encoding: 'terrarium', tileSize: 256, maxzoom: 15 };
    style.terrain = { source: 'dem', exaggeration: mp.exaggeration };
  }
  if (mp.buildings) {
    sources.ofm = { type: 'vector', url: OFM_TILEJSON };
    layers.push(buildingLayer('ofm'));
  }
  return { style };
}
function addExtrasToVectorStyle(map, projection) {
  try {
    map.setProjection(projection);
    map.setSky(SKY);
    if (mp.terrain) {
      if (!map.getSource('dem')) map.addSource('dem', { type: 'raster-dem', tiles: [DEM_TILES], encoding: 'terrarium', tileSize: 256, maxzoom: 15 });
      map.setTerrain({ source: 'dem', exaggeration: mp.exaggeration });
    }
    const style = map.getStyle();
    const extrusions = style.layers.filter(l => l.type === 'fill-extrusion');
    extrusions.forEach(l => map.setLayoutProperty(l.id, 'visibility', mp.buildings ? 'visible' : 'none'));
    if (mp.buildings && !extrusions.length) {
      const vec = Object.keys(style.sources).find(k => style.sources[k].type === 'vector');
      if (vec) map.addLayer(buildingLayer(vec));
    }
  } catch (e) { console.warn(e); }
}
async function mpCreateMap(container, R, { interactive = false, preserve = false } = {}) {
  if (!window.maplibregl) throw Object.assign(new Error('The map engine didn’t load.'), { code: 'nolib' });
  const st = await mpStyle(mp.source, R);
  const cam = mpCamera(0);
  const map = new maplibregl.Map({
    container, style: st.url || st.style,
    center: cam.center, zoom: cam.zoom, bearing: cam.bearing, pitch: cam.pitch,
    pixelRatio: R, interactive, attributionControl: false, fadeDuration: 0, maxPitch: 75,
    maxTileCacheSize: 1500,
    canvasContextAttributes: { preserveDrawingBuffer: preserve, antialias: true },
    transformRequest: url => { countTile(url); return { url }; },
  });
  if (st.url) map.once('style.load', () => addExtrasToVectorStyle(map, st.projection));
  return map;
}

/* ----- geography helpers ----- */
const D2R = Math.PI / 180;
const mercX = lng => (lng + 180) / 360;
const mercY = lat => { const s = Math.sin(clamp(lat, -85.05, 85.05) * D2R); return 0.5 - Math.log((1 + s) / (1 - s)) / (4 * Math.PI); };
const lngOfX = x => x * 360 - 180;
const latOfY = y => Math.atan(Math.sinh(Math.PI - 2 * Math.PI * clamp(y, 0.0005, 0.9995))) / D2R;
function haversineKm(a, b) {
  const dLat = (b.lat - a.lat) * D2R, dLng = (b.lng - a.lng) * D2R;
  const h = Math.sin(dLat / 2) ** 2 + Math.cos(a.lat * D2R) * Math.cos(b.lat * D2R) * Math.sin(dLng / 2) ** 2;
  return 2 * 6371 * Math.asin(Math.min(1, Math.sqrt(h)));
}
function greatCircle(a, b, n) {
  const v = p => [Math.cos(p.lat * D2R) * Math.cos(p.lng * D2R), Math.cos(p.lat * D2R) * Math.sin(p.lng * D2R), Math.sin(p.lat * D2R)];
  const v1 = v(a), v2 = v(b);
  const d = Math.acos(clamp(v1[0] * v2[0] + v1[1] * v2[1] + v1[2] * v2[2], -1, 1));
  if (d < 1e-9) return [[a.lng, a.lat], [b.lng, b.lat]];
  const pts = [];
  for (let i = 0; i <= n; i++) {
    const f = i / n, A = Math.sin((1 - f) * d) / Math.sin(d), B = Math.sin(f * d) / Math.sin(d);
    const x = A * v1[0] + B * v2[0], y = A * v1[1] + B * v2[1], z = A * v1[2] + B * v2[2];
    pts.push([Math.atan2(y, x) / D2R, Math.atan2(z, Math.hypot(x, y)) / D2R]);
  }
  return pts;
}
function mercArc(a, b, bend, n) {
  const ax = mercX(a.lng), ay = mercY(a.lat);
  let bx = mercX(b.lng); const by = mercY(b.lat);
  if (bx - ax > 0.5) bx -= 1; else if (bx - ax < -0.5) bx += 1;
  const dx = bx - ax, dy = by - ay, len = Math.hypot(dx, dy) || 1e-9;
  let px = -dy / len, py = dx / len;
  if (py > 0) { px = -px; py = -py; } // bulge toward the pole side
  const cx = (ax + bx) / 2 + px * len * bend * 2, cy = (ay + by) / 2 + py * len * bend * 2;
  const pts = [];
  for (let i = 0; i <= n; i++) {
    const t = i / n, u = 1 - t;
    const x = u * u * ax + 2 * u * t * cx + t * t * bx, y = u * u * ay + 2 * u * t * cy + t * t * by;
    pts.push([lngOfX(x), latOfY(y)]);
  }
  return pts;
}
/* Route points in order: From, any stops that have been looked up, To */
const routePoints = () => (mp.rA && mp.rB ? [mp.rA, ...mp.rStops.filter(s => Number.isFinite(s.lat) && Number.isFinite(s.lng)), mp.rB] : []);
const roadKey = () => routePoints().map(p => `${p.lat.toFixed(5)},${p.lng.toFixed(5)}`).join('|');
let routeGeo = null;
function buildRoute() {
  const P = routePoints();
  if (P.length < 2) { routeGeo = null; return; }
  const isRoad = mp.rPath === 'roads' && Array.isArray(mp.rRoad) && mp.rRoad.length > 1 && mp.rRoadKey === roadKey();
  // Arcs and flight paths are drawn leg by leg; legStart[k] is the index where leg k begins
  let pts = [];
  const legStart = [];
  if (isRoad) pts = mp.rRoad.map(p => [p[0], p[1]]);
  else {
    for (let k = 0; k < P.length - 1; k++) {
      const leg = mp.rPath === 'great' ? greatCircle(P[k], P[k + 1], 256) : mercArc(P[k], P[k + 1], mp.rPath === 'roads' ? 0 : mp.rBend, 200);
      legStart.push(Math.max(0, pts.length - 1));
      pts.push(...(k ? leg.slice(1) : leg));
    }
  }
  for (let i = 1; i < pts.length; i++) {
    while (pts[i][0] - pts[i - 1][0] > 180) pts[i][0] -= 360;
    while (pts[i][0] - pts[i - 1][0] < -180) pts[i][0] += 360;
  }
  const cum = [0];
  let minx = Infinity, maxx = -Infinity, miny = Infinity, maxy = -Infinity;
  pts.forEach((p, i) => {
    const x = mercX(p[0]), y = mercY(p[1]);
    minx = Math.min(minx, x); maxx = Math.max(maxx, x); miny = Math.min(miny, y); maxy = Math.max(maxy, y);
    if (i) { const q = pts[i - 1]; cum.push(cum[i - 1] + Math.hypot(x - mercX(q[0]), y - mercY(q[1]))); }
  });
  let straight = 0;
  for (let k = 0; k < P.length - 1; k++) straight += haversineKm(P[k], P[k + 1]);
  const distKm = isRoad && mp.rRoadDist ? mp.rRoadDist / 1000 : straight;
  const total = cum[cum.length - 1] || 1e-9;
  // Where each point sits along the line (0..1). A road line only passes near its stops, so take the closest vertex.
  let from = 0;
  const stops = P.map((p, k) => {
    let idx;
    if (k === 0) idx = 0;
    else if (k === P.length - 1) idx = pts.length - 1;
    else if (!isRoad) idx = legStart[k];
    else {
      let best = Infinity; idx = from;
      for (let j = from; j < pts.length; j++) {
        const d = Math.hypot(mercX(pts[j][0]) - mercX(p.lng), mercY(pts[j][1]) - mercY(p.lat));
        if (d < best) { best = d; idx = j; }
      }
    }
    from = idx;
    return { frac: clamp(cum[idx] / total, 0, 1), lng: pts[idx][0], lat: pts[idx][1], label: k === 0 || k === P.length - 1 ? '' : mp.rStops.filter(s => Number.isFinite(s.lat))[k - 1]?.label || '' };  // ends use rLabelA / rLabelB, which can change later
  });
  routeGeo = { pts, cum, total, bounds: { minx, maxx, miny, maxy }, distKm, isRoad, stops };
}
function routeFit() {
  const b = routeGeo.bounds, { LW, LH } = mpLogical();
  const sx = Math.max(1e-7, b.maxx - b.minx), sy = Math.max(1e-7, b.maxy - b.miny);
  const z = Math.min(Math.log2(LW * 0.64 / (256 * sx)), Math.log2(LH * 0.58 / (256 * sy)));
  return { lng: lngOfX((b.minx + b.maxx) / 2), lat: latOfY((b.miny + b.maxy) / 2), zoom: clamp(z, 1.2, 16) };
}
function headAt(p) {
  const G = routeGeo, target = p * G.total;
  let i = 1;
  while (i < G.cum.length - 1 && G.cum[i] < target) i++;
  const a = G.pts[i - 1], b = G.pts[i];
  const seg = G.cum[i] - G.cum[i - 1] || 1e-12;
  const f = clamp((target - G.cum[i - 1]) / seg, 0, 1);
  return { lng: lerp(a[0], b[0], f), lat: lerp(a[1], b[1], f), i };
}
function fmtDist(km) {
  const mi = km * 0.621371;
  const f = v => v < 100 ? v.toFixed(1) : Math.round(v).toLocaleString();
  return `${f(km)} km · ${f(mi)} mi`;
}

/* ----- camera over time. Zoom values are Google-style (as in a Maps link); MapLibre's scale is one level lower ----- */
const midStops = () => (routeGeo ? routeGeo.stops.length - 2 : 0);
function routeTimes() {
  const fly = mp.rFlyIn ? mp.duration : 0;
  const pauses = mp.rPause > 0 ? mp.rPause * midStops() : 0;
  const drawStart = mp.delay + fly, drawEnd = drawStart + mp.rDraw + pauses;
  return { fly, drawStart, drawEnd, pauses, total: drawEnd + mp.hold };
}
/* Line progress (0..1) at time t. With stops and a pause, each leg eases into its stop, waits, and moves on;
   leg k takes a share of the draw time equal to its share of the length. */
function routeProgress(t) {
  const { drawStart } = routeTimes();
  const D = Math.max(0.1, mp.rDraw);
  if (!midStops() || mp.rPause <= 0) return ease(mp.ease, (t - drawStart) / D);
  const f = routeGeo.stops.map(s => s.frac);
  let u = t - drawStart;
  for (let k = 0; k < f.length - 1; k++) {
    const d = Math.max(0.05, D * (f[k + 1] - f[k]));
    if (u < d) return f[k] + (f[k + 1] - f[k]) * ease(mp.ease, u / d);
    u -= d;
    if (k < f.length - 2) { if (u < mp.rPause) return f[k + 1]; u -= mp.rPause; }
  }
  return 1;
}
/* When the line reaches each stop (searched, since the eased progress has no simple inverse) */
function stopTimes() {
  const { drawStart, drawEnd } = routeTimes();
  return routeGeo.stops.map(s => {
    let lo = drawStart, hi = drawEnd;
    for (let i = 0; i < 32; i++) { const mid = (lo + hi) / 2; if (routeProgress(mid) >= s.frac - 1e-6) hi = mid; else lo = mid; }
    return hi;
  });
}
function placeCamera(t, end = mp.endCenter || mp.target, endZoom = mp.endZoom) {
  const u = clamp((t - mp.delay) / Math.max(0.1, mp.duration), 0, 1);
  const e = ease(mp.ease, u);
  const zoom = lerp(Math.min(mp.startZoom, endZoom), endZoom, e) - 1;
  const ce = 1 - Math.pow(1 - clamp(u / 0.6, 0, 1), 3);
  const lng = lerp(end.lng - mp.spin, end.lng, ce);
  const lat = lerp(clamp(end.lat * 0.75, -60, 60), end.lat, ce);
  return { center: [lng, lat], zoom, bearing: lerp(0, mp.rotate, e), pitch: mp.pitch * smoothstep((u - 0.5) / 0.5) };
}
function routeCamera(t) {
  if (!routeGeo) return placeCamera(t);
  const { drawStart, drawEnd } = routeTimes();
  const fit = routeFit();
  const bump = Math.max(0, mp.rFollowZoom - fit.zoom) * 0.85;
  const followAt = p => { const h = headAt(p); return { lng: h.lng, lat: h.lat, zoom: mp.rFollowZoom - bump * Math.sin(Math.PI * p) }; };
  const first = mp.rCam === 'follow' ? followAt(0) : fit;
  let cam;
  if (t < drawStart) {
    if (mp.rFlyIn) {
      const u = clamp((t - mp.delay) / Math.max(0.1, mp.duration), 0, 1);
      const e = ease(mp.ease, u), ce = 1 - Math.pow(1 - clamp(u / 0.6, 0, 1), 3);
      cam = { lng: lerp(first.lng - mp.spin, first.lng, ce), lat: lerp(clamp(first.lat * 0.75, -60, 60), first.lat, ce), zoom: lerp(Math.min(mp.startZoom, first.zoom), first.zoom, e) };
    } else cam = first;
  } else {
    cam = mp.rCam === 'follow' ? followAt(routeProgress(t)) : fit;
  }
  const whole = clamp(t / Math.max(0.1, drawEnd), 0, 1);
  return { center: [cam.lng, cam.lat], zoom: cam.zoom - 1, bearing: lerp(0, mp.rotate, EASE.smooth(whole)), pitch: mp.pitch * smoothstep((whole - 0.35) / 0.65) };
}
/* Outline mode: fly in like a place zoom, framed to fit the area, then draw its lines */
const AREA_COLORS = ['#0A84FF', '#FF3B30', '#34C759', '#BF5AF2', '#FF9F0A', '#64D2FF', '#FF375F'];
const areaColor = i => mp.areas[i]?.color || (i === 0 ? mp.aColor : AREA_COLORS[(i - 1) % AREA_COLORS.length]);
/* The frame that fits every area: the union of their bounds, with a little extra room for labels */
let frameCache = { key: '', v: null };
function areasFrame() {
  const key = mp.areas.map(a => a.id).join('|') + mp.out.aspect;
  if (frameCache.key !== key) {
    const b = mp.areas.length === 1 ? mp.areas[0].bbox : CS.boundaries.bounds(mp.areas.flatMap(a => a.rings));
    const mid = (b[1] + b[3]) / 2;
    frameCache = { key, v: { bbox: b, center: { lat: (b[0] + b[2]) / 2, lng: mid > 180 ? mid - 360 : mid }, zoom: zoomForBox(b) - 0.25 } };
  }
  return frameCache.v;
}
/* Each area draws in turn ("one after another", overlapping a little) or all together */
function areaTimes() {
  const n = Math.max(1, mp.areas.length), D = Math.max(0.2, mp.aDraw);
  const step = mp.aOrder === 'seq' ? D * 0.75 : 0;
  const drawStart = mp.delay + mp.duration, drawEnd = drawStart + (n - 1) * step + D;
  return { drawStart, drawEnd, step, starts: Array.from({ length: n }, (_, i) => drawStart + i * step), total: drawEnd + mp.hold };
}
const areaCamera = t => (mp.areas.length ? placeCamera(t, areasFrame().center, areasFrame().zoom) : placeCamera(t));
const mpCamera = t => (mp.mode === 'route' ? routeCamera(t) : mp.mode === 'area' ? areaCamera(t) : placeCamera(t));
const mpTotal = () => (mp.mode === 'route' ? routeTimes().total : mp.mode === 'area' ? areaTimes().total : mp.delay + mp.duration + mp.hold);
const mpMarkerStart = () => (mp.mode === 'route' ? routeTimes().drawEnd : mp.mode === 'area' ? areaTimes().drawEnd - Math.min(0.5, mp.aDraw * 0.3) : mp.delay + mp.duration * 0.9);

/* ----- overlays: route line, markers, labels, vignette, credit. `s` = output pixels per logical pixel ----- */
function fmtCoords(lat, lng) { return `${Math.abs(lat).toFixed(4)}° ${lat >= 0 ? 'N' : 'S'}, ${Math.abs(lng).toFixed(4)}° ${lng >= 0 ? 'E' : 'W'}`; }
function drawStars(ctx, W, H, s) {
  const rnd = mulberry32(20261014);
  const n = Math.round((W / s) * (H / s) / 1400);
  ctx.save();
  for (let i = 0; i < n; i++) {
    const x = rnd() * W, y = rnd() * H, r = (0.35 + rnd() * rnd() * 1.1) * s, a = 0.25 + rnd() * 0.7;
    ctx.globalAlpha = a; ctx.fillStyle = rnd() > 0.85 ? '#cfe0ff' : '#ffffff';
    ctx.beginPath(); ctx.arc(x, y, r, 0, Math.PI * 2); ctx.fill();
  }
  ctx.restore();
}
function drawPin(ctx, x, y, k, s, color) {
  const drop = clamp(k / 0.55, 0, 1);
  const fall = (1 - easeOutBounce(drop)) * 90 * s;
  const a = clamp(k / 0.12, 0, 1);
  ctx.save();
  ctx.globalAlpha = 0.35 * a * (0.4 + 0.6 * drop);
  ctx.fillStyle = '#000';
  ctx.beginPath(); ctx.ellipse(x, y, 9 * s * (0.5 + 0.5 * drop), 3.2 * s * (0.5 + 0.5 * drop), 0, 0, Math.PI * 2); ctx.fill();
  ctx.globalAlpha = a;
  ctx.translate(x, y - fall);
  const R = 15 * s, cy = -38 * s;
  ctx.shadowColor = 'rgba(0,0,0,0.35)'; ctx.shadowBlur = 8 * s; ctx.shadowOffsetY = 3 * s;
  ctx.fillStyle = color;
  ctx.beginPath();
  ctx.moveTo(0, 0);
  ctx.bezierCurveTo(-4 * s, -10 * s, -R, cy + R * 0.75, -R, cy);
  ctx.arc(0, cy, R, Math.PI, 0);
  ctx.bezierCurveTo(R, cy + R * 0.75, 4 * s, -10 * s, 0, 0);
  ctx.closePath(); ctx.fill();
  ctx.shadowColor = 'transparent';
  ctx.lineWidth = 1.5 * s; ctx.strokeStyle = 'rgba(0,0,0,0.18)'; ctx.stroke();
  ctx.fillStyle = '#fff'; ctx.beginPath(); ctx.arc(0, cy, 5.5 * s, 0, Math.PI * 2); ctx.fill();
  ctx.restore();
  return 54 * s;
}
function drawPulse(ctx, x, y, k, s, color) {
  const a = clamp(k / 0.2, 0, 1);
  ctx.save();
  for (let n = 0; n < 3; n++) {
    const raw = k * 0.75 - n / 3;
    if (raw < 0) continue;
    const ph = raw % 1;
    ctx.globalAlpha = a * (1 - ph) * 0.85; ctx.strokeStyle = color; ctx.lineWidth = 3 * s;
    ctx.beginPath(); ctx.arc(x, y, 9 * s + 48 * s * ph, 0, Math.PI * 2); ctx.stroke();
  }
  ctx.globalAlpha = a;
  ctx.fillStyle = color; ctx.beginPath(); ctx.arc(x, y, 9 * s * easeOutBack(clamp(k / 0.35, 0, 1)), 0, Math.PI * 2); ctx.fill();
  ctx.lineWidth = 3 * s; ctx.strokeStyle = '#fff'; ctx.stroke();
  ctx.restore();
  return 26 * s;
}
function drawTarget(ctx, x, y, k, s, color) {
  const lock = EASE.snappy(clamp(k / 0.7, 0, 1));
  const r = lerp(160, 34, lock) * s * (1 + 0.04 * Math.sin(Math.max(0, k - 0.7) * 4));
  ctx.save();
  ctx.globalAlpha = clamp(k / 0.15, 0, 1);
  ctx.strokeStyle = color; ctx.lineWidth = 3.5 * s;
  ctx.beginPath(); ctx.arc(x, y, r, 0, Math.PI * 2); ctx.stroke();
  ctx.lineWidth = 3 * s;
  for (let i = 0; i < 4; i++) {
    const ang = i * Math.PI / 2 + lerp(Math.PI / 4, 0, lock);
    ctx.beginPath(); ctx.moveTo(x + Math.cos(ang) * (r - 8 * s), y + Math.sin(ang) * (r - 8 * s)); ctx.lineTo(x + Math.cos(ang) * (r + 12 * s), y + Math.sin(ang) * (r + 12 * s)); ctx.stroke();
  }
  ctx.fillStyle = color; ctx.beginPath(); ctx.arc(x, y, 4.5 * s * lock, 0, Math.PI * 2); ctx.fill();
  ctx.restore();
  return r + 14 * s;
}
function drawMarker(ctx, x, y, k, s) {
  if (mp.marker === 'pin') return drawPin(ctx, x, y, k, s, mp.markerColor);
  if (mp.marker === 'pulse') return drawPulse(ctx, x, y, k, s, mp.markerColor);
  if (mp.marker === 'target') return drawTarget(ctx, x, y, k, s, mp.markerColor);
  return 12 * s;
}
function drawPillLabel(ctx, x, y, k, s, lift, text) {
  if (k <= 0 || !text || !text.trim()) return;
  const e = EASE.snappy(clamp(k / 0.35, 0, 1));
  ctx.save();
  ctx.globalAlpha = e;
  ctx.font = `600 ${Math.round(21 * s)}px "Barlow", "Helvetica Neue", Arial, sans-serif`;
  const tw = ctx.measureText(text).width;
  const ph = 38 * s, pw = tw + 30 * s;
  const cx = x, by = y - lift - 10 * s + (1 - e) * 10 * s;
  ctx.shadowColor = 'rgba(0,0,0,0.35)'; ctx.shadowBlur = 14 * s; ctx.shadowOffsetY = 4 * s;
  ctx.fillStyle = mp.labelBg;
  roundRect(ctx, cx - pw / 2, by - ph, pw, ph, ph / 2); ctx.fill();
  ctx.beginPath(); ctx.moveTo(cx - 7 * s, by - 1); ctx.lineTo(cx + 7 * s, by - 1); ctx.lineTo(cx, by + 7 * s); ctx.closePath(); ctx.fill();
  ctx.shadowColor = 'transparent';
  ctx.fillStyle = mp.labelFg; ctx.textAlign = 'center'; ctx.textBaseline = 'middle';
  ctx.fillText(text, cx, by - ph / 2 + 1 * s);
  ctx.restore();
}
function drawLowerThird(ctx, W, H, k, s, title, sub) {
  if (k <= 0 || !title || !title.trim()) return;
  const e = EASE.snappy(clamp(k / 0.45, 0, 1));
  const x = 34 * s, b = H - 40 * s;
  ctx.save();
  ctx.font = `700 ${Math.round(26 * s)}px "Barlow Condensed", "Arial Narrow", Arial, sans-serif`;
  const nw = ctx.measureText(title.toUpperCase()).width;
  ctx.font = `500 ${Math.round(14 * s)}px "IBM Plex Mono", Menlo, monospace`;
  const cw = sub ? ctx.measureText(sub).width : 0;
  const w = Math.max(nw, cw) + 40 * s, h = (sub ? 70 : 52) * s;
  ctx.beginPath(); ctx.rect(x, b - h, w * e + 6 * s, h); ctx.clip();
  ctx.globalAlpha = 0.94; ctx.fillStyle = mp.labelBg; ctx.fillRect(x + 6 * s, b - h, w, h);
  ctx.globalAlpha = 1; ctx.fillStyle = mp.markerColor; ctx.fillRect(x, b - h, 6 * s, h);
  ctx.fillStyle = mp.labelFg; ctx.textBaseline = 'alphabetic';
  ctx.font = `700 ${Math.round(26 * s)}px "Barlow Condensed", "Arial Narrow", Arial, sans-serif`;
  ctx.fillText(title.toUpperCase(), x + 24 * s, b - h + 36 * s);
  if (sub) {
    ctx.globalAlpha = 0.7;
    ctx.font = `500 ${Math.round(14 * s)}px "IBM Plex Mono", Menlo, monospace`;
    ctx.fillText(sub, x + 24 * s, b - h + 58 * s);
  }
  ctx.restore();
}
function drawCredit(ctx, W, H, s, text) {
  if (!text) return;
  ctx.save();
  ctx.font = `500 ${Math.round(10.5 * s)}px "Barlow", Arial, sans-serif`;
  ctx.textAlign = 'right'; ctx.textBaseline = 'alphabetic';
  ctx.shadowColor = 'rgba(0,0,0,0.7)'; ctx.shadowBlur = 3 * s;
  ctx.fillStyle = 'rgba(255,255,255,0.9)';
  // Wrap long credits at the " · " separators so they never run off the left edge
  const maxW = W - 18 * s, lines = [];
  for (const part of text.split(' · ')) {
    const cur = lines[lines.length - 1];
    if (cur && ctx.measureText(cur + ' · ' + part).width <= maxW) lines[lines.length - 1] = cur + ' · ' + part;
    else lines.push(part);
  }
  lines.reverse().forEach((ln, i) => ctx.fillText(ln, W - 9 * s, H - 9 * s - i * 13 * s));
  ctx.restore();
}
function drawPlane(ctx, x, y, ang, size, color) {
  ctx.save(); ctx.translate(x, y); ctx.rotate(ang); const k = size / 24; ctx.scale(k, k);
  ctx.beginPath();
  ctx.moveTo(0, -12); ctx.bezierCurveTo(1.6, -11, 1.9, -8, 1.9, -5); ctx.lineTo(1.9, -1.5); ctx.lineTo(11, 3.5); ctx.lineTo(11, 5.6);
  ctx.lineTo(1.9, 3.2); ctx.lineTo(1.6, 8); ctx.lineTo(4.6, 10.2); ctx.lineTo(4.6, 11.8); ctx.lineTo(0, 10.6); ctx.lineTo(-4.6, 11.8);
  ctx.lineTo(-4.6, 10.2); ctx.lineTo(-1.6, 8); ctx.lineTo(-1.9, 3.2); ctx.lineTo(-11, 5.6); ctx.lineTo(-11, 3.5); ctx.lineTo(-1.9, -1.5);
  ctx.lineTo(-1.9, -5); ctx.bezierCurveTo(-1.9, -8, -1.6, -11, 0, -12); ctx.closePath();
  ctx.shadowColor = 'rgba(0,0,0,0.45)'; ctx.shadowBlur = 6 / k; ctx.shadowOffsetY = 2 / k;
  ctx.fillStyle = '#ffffff'; ctx.fill();
  ctx.shadowColor = 'transparent'; ctx.lineWidth = 1.2; ctx.strokeStyle = color; ctx.stroke();
  ctx.restore();
}
function drawRoute(ctx, W, H, s, t, view) {
  const G = routeGeo; if (!G) return;
  const { drawStart, drawEnd } = routeTimes();
  const p = t < drawStart ? 0 : routeProgress(t);
  const lw = mp.lineWidth * s;
  // line
  if (p > 0) {
    const h = headAt(p);
    const path = G.pts.slice(0, h.i);
    path.push([h.lng, h.lat]);
    const runs = []; let cur = [];
    for (const ll of path) {
      const q = view.project(ll);
      if (!q) { if (cur.length > 1) runs.push(cur); cur = []; continue; }
      cur.push(q);
    }
    if (cur.length > 1) runs.push(cur);
    ctx.save();
    ctx.lineCap = 'round'; ctx.lineJoin = 'round';
    const strokeRuns = () => { for (const r of runs) { ctx.beginPath(); r.forEach((q, i) => (i ? ctx.lineTo(q.x, q.y) : ctx.moveTo(q.x, q.y))); ctx.stroke(); } };
    if (mp.lineGlow) { ctx.strokeStyle = mp.lineColor; ctx.globalAlpha = 0.28; ctx.lineWidth = lw * 3.2; strokeRuns(); ctx.globalAlpha = 1; }
    ctx.strokeStyle = 'rgba(0,0,0,0.35)'; ctx.lineWidth = lw + 2 * s; if (mp.lineDash) ctx.setLineDash([lw * 2.6, lw * 2]); strokeRuns();
    ctx.strokeStyle = mp.lineColor; ctx.lineWidth = lw; strokeRuns();
    ctx.setLineDash([]);
    // moving icon
    const fade = 1 - clamp((t - drawEnd) / 0.3, 0, 1);
    const last = runs[runs.length - 1];
    if (mp.rIcon !== 'none' && fade > 0 && last && last.length > 1) {
      const hq = last[last.length - 1];
      let k = last.length - 2; while (k > 0 && Math.hypot(hq.x - last[k].x, hq.y - last[k].y) < 6 * s) k--;
      const pq = last[k];
      const ang = Math.atan2(hq.y - pq.y, hq.x - pq.x);
      ctx.globalAlpha = fade;
      if (mp.rIcon === 'plane') drawPlane(ctx, hq.x, hq.y, ang + Math.PI / 2, 30 * s, mp.lineColor);
      else if (mp.rIcon === 'arrow') {
        ctx.save(); ctx.translate(hq.x, hq.y); ctx.rotate(ang);
        ctx.fillStyle = mp.lineColor; ctx.strokeStyle = '#fff'; ctx.lineWidth = 2 * s;
        ctx.beginPath(); ctx.moveTo(lw * 1.2 + 10 * s, 0); ctx.lineTo(-8 * s, 9 * s + lw * 0.5); ctx.lineTo(-3 * s, 0); ctx.lineTo(-8 * s, -9 * s - lw * 0.5); ctx.closePath(); ctx.fill(); ctx.stroke();
        ctx.restore();
      } else {
        ctx.fillStyle = '#fff'; ctx.beginPath(); ctx.arc(hq.x, hq.y, lw * 0.9 + 5 * s, 0, Math.PI * 2); ctx.fill();
        ctx.fillStyle = mp.lineColor; ctx.beginPath(); ctx.arc(hq.x, hq.y, lw * 0.9 + 2.5 * s, 0, Math.PI * 2); ctx.fill();
      }
    }
    ctx.restore();
  }
  // start point
  const appear = t - (drawStart - 0.35);
  if (appear > 0) {
    const qa = view.project(G.pts[0]);
    if (qa) {
      const e = easeOutBack(clamp(appear / 0.35, 0, 1));
      ctx.save();
      ctx.fillStyle = '#fff'; ctx.beginPath(); ctx.arc(qa.x, qa.y, (7 + lw / s * 0.6) * s * e, 0, Math.PI * 2); ctx.fill();
      ctx.fillStyle = mp.lineColor; ctx.beginPath(); ctx.arc(qa.x, qa.y, (4.5 + lw / s * 0.6) * s * e, 0, Math.PI * 2); ctx.fill();
      ctx.restore();
      if (mp.labelStyle === 'pill') drawPillLabel(ctx, qa.x, qa.y, appear - 0.15, s, 14 * s, mp.rLabelA);
    }
  }
  // stops along the way pop in as the line reaches them
  if (G.stops.length > 2) {
    const when = stopTimes();
    for (let j = 1; j < G.stops.length - 1; j++) {
      const age = t - when[j];
      if (age <= 0) continue;
      const q = view.project([G.stops[j].lng, G.stops[j].lat]);
      if (!q) continue;
      const e = easeOutBack(clamp(age / 0.35, 0, 1));
      ctx.save();
      ctx.fillStyle = '#fff'; ctx.beginPath(); ctx.arc(q.x, q.y, (7 + lw / s * 0.6) * s * e, 0, Math.PI * 2); ctx.fill();
      ctx.fillStyle = mp.lineColor; ctx.beginPath(); ctx.arc(q.x, q.y, (4.5 + lw / s * 0.6) * s * e, 0, Math.PI * 2); ctx.fill();
      ctx.restore();
      if (mp.labelStyle === 'pill' && G.stops[j].label) drawPillLabel(ctx, q.x, q.y, age - 0.1, s, 14 * s, G.stops[j].label);
    }
  }
  // destination
  const k = t - drawEnd;
  if (k > 0) {
    const qb = view.project(G.pts[G.pts.length - 1]);
    if (qb) {
      const lift = drawMarker(ctx, qb.x, qb.y, k, s);
      if (mp.labelStyle === 'pill') drawPillLabel(ctx, qb.x, qb.y, k - 0.3, s, lift, mp.rLabelB);
    }
  }
}
/* Outlines: each area's border draws itself along every ring at once, then its tint fades in. Everything outside the
   areas dims, with each area cut out of the dim as it is drawn (on a layer, so overlapping areas don't cancel out). */
const dimLayer = document.createElement('canvas');
function areaProgress(i, t) {
  const t0 = areaTimes().starts[i], D = Math.max(0.2, mp.aDraw);
  return { p: ease(mp.ease, (t - t0) / D), fp: EASE.smooth(clamp((t - t0 - D * 0.55) / (D * 0.6), 0, 1)), done: t - (t0 + D) };
}
function drawAreas(ctx, W, H, s, t, view) {
  if (!mp.areas.length) return;
  // Project every ring; a ring that dips behind the globe is drawn in pieces and left out of fills
  const shapes = mp.areas.map((A, i) => {
    let whole = true;
    const rings = A.rings.map(r => {
      const runs = []; let cur = [];
      for (const ll of r) { const q = view.project(ll); if (!q) { whole = false; if (cur.length > 1) runs.push(cur); cur = []; } else cur.push(q); }
      if (cur.length > 1) runs.push(cur);
      return runs;
    });
    return { A, i, rings, whole, ...areaProgress(i, t) };
  });
  const trace = (g, rings) => { g.beginPath(); for (const runs of rings) for (const run of runs) { run.forEach((q, k) => (k ? g.lineTo(q.x, q.y) : g.moveTo(q.x, q.y))); g.closePath(); } };
  const dimA = mp.aDim * shapes[0].fp;
  if (dimA > 0.001 && shapes.every(sh => sh.whole)) {
    const cw = ctx.canvas.width, ch = ctx.canvas.height;
    if (dimLayer.width !== cw || dimLayer.height !== ch) { dimLayer.width = cw; dimLayer.height = ch; }
    const g = dimLayer.getContext('2d');
    g.setTransform(1, 0, 0, 1, 0, 0); g.clearRect(0, 0, cw, ch);
    g.setTransform(ctx.getTransform());
    g.globalCompositeOperation = 'source-over'; g.globalAlpha = 1; g.fillStyle = `rgba(0,0,0,${dimA})`; g.fillRect(0, 0, W, H);
    g.globalCompositeOperation = 'destination-out';
    for (const sh of shapes) if (sh.fp > 0) { g.globalAlpha = sh.fp; trace(g, sh.rings); g.fillStyle = '#000'; g.fill('evenodd'); }
    ctx.save(); ctx.setTransform(1, 0, 0, 1, 0, 0); ctx.drawImage(dimLayer, 0, 0); ctx.restore();
  }
  const lw = mp.aWidth * s;
  ctx.save();
  ctx.lineJoin = 'round'; ctx.lineCap = 'round';
  for (const sh of shapes) {
    if (sh.p <= 0) continue;
    const color = areaColor(sh.i);
    if (mp.aFill > 0 && sh.fp > 0 && sh.whole) { trace(ctx, sh.rings); ctx.globalAlpha = mp.aFill * sh.fp; ctx.fillStyle = color; ctx.fill('evenodd'); ctx.globalAlpha = 1; }
    // Each run draws from its start; dashing by its own length gives the "drawing on" look
    for (const runs of sh.rings) for (const run of runs) {
      let len = 0; for (let k = 1; k < run.length; k++) len += Math.hypot(run[k].x - run[k - 1].x, run[k].y - run[k - 1].y);
      ctx.setLineDash(sh.p < 1 ? [len * sh.p, len + 1] : []);
      ctx.beginPath(); run.forEach((q, k) => (k ? ctx.lineTo(q.x, q.y) : ctx.moveTo(q.x, q.y)));
      ctx.globalAlpha = 0.3; ctx.strokeStyle = color; ctx.lineWidth = lw * 3; ctx.stroke();
      ctx.globalAlpha = 1; ctx.strokeStyle = 'rgba(0,0,0,0.4)'; ctx.lineWidth = lw + 2 * s; ctx.stroke();
      ctx.strokeStyle = color; ctx.lineWidth = lw; ctx.stroke();
    }
    ctx.setLineDash([]);
    // A radius gets a dot at its center
    if (sh.A.mark) {
      const q = view.project([sh.A.mark.lng, sh.A.mark.lat]);
      if (q) { const e = easeOutBack(clamp(sh.p * 1.5, 0, 1)); ctx.fillStyle = '#fff'; ctx.beginPath(); ctx.arc(q.x, q.y, 8 * s * e, 0, Math.PI * 2); ctx.fill(); ctx.fillStyle = color; ctx.beginPath(); ctx.arc(q.x, q.y, 5 * s * e, 0, Math.PI * 2); ctx.fill(); }
    }
  }
  ctx.restore();
}
function mpOverlays(ctx, W, H, s, t, view, credit) {
  if (mp.vignette) {
    const g = ctx.createRadialGradient(W / 2, H / 2, Math.min(W, H) * 0.35, W / 2, H / 2, Math.hypot(W, H) * 0.6);
    g.addColorStop(0, 'rgba(0,0,0,0)'); g.addColorStop(1, 'rgba(0,0,0,0.45)');
    ctx.fillStyle = g; ctx.fillRect(0, 0, W, H);
  }
  const k = t - mpMarkerStart();
  if (mp.mode === 'route') {
    drawRoute(ctx, W, H, s, t, view);
    if (mp.labelStyle === 'lower' && routeGeo) drawLowerThird(ctx, W, H, k - 0.15, s, mp.rTitle, mp.coords ? fmtDist(routeGeo.distKm) : '');
  } else if (mp.mode === 'area') {
    drawAreas(ctx, W, H, s, t, view);
    // Each area's label appears as its own border finishes, centered on the area rather than pointing at a spot
    if (mp.labelStyle === 'pill') mp.areas.forEach((A, i) => {
      const at = A.mark || A.labelAt, q = view.project([at.lng, at.lat]);
      if (q) drawPillLabel(ctx, q.x, q.y + (A.mark ? -10 : 24) * s, areaProgress(i, t).done + 0.25, s, 0, A.label);
    });
    if (mp.areas.length && mp.labelStyle === 'lower') drawLowerThird(ctx, W, H, k - 0.15, s, mp.aTitle, mp.coords ? (mp.areas.length === 1 ? mp.areas[0].detail.split(' · ')[0] : `${mp.areas.length} areas`) : '');
  } else {
    const p = view.project([mp.target.lng, mp.target.lat]);
    if (p && k > 0) {
      const lift = drawMarker(ctx, p.x, p.y, k, s);
      if (mp.labelStyle === 'pill') drawPillLabel(ctx, p.x, p.y, k - 0.3, s, lift, mp.label);
    }
    if (mp.labelStyle === 'lower') drawLowerThird(ctx, W, H, k - 0.15, s, mp.label, mp.coords ? fmtCoords(mp.target.lat, mp.target.lng) : '');
  }
  drawCredit(ctx, W, H, s, credit);
}
/* Projects lng/lat into canvas pixels; returns null when the point is behind the globe */
function mapView(map, factor) {
  const c = map.getCenter();
  return {
    project(ll) {
      const lng = ll[0] + 360 * Math.round((c.lng - ll[0]) / 360);
      try { if (map.transform?.isLocationOccluded?.(new maplibregl.LngLat(lng, ll[1]))) return null; } catch (e) { /* older engine */ }
      const p = map.project([lng, ll[1]]);
      return isFinite(p.x) && isFinite(p.y) ? { x: p.x * factor, y: p.y * factor } : null;
    },
  };
}

/* ----- preview map ----- */
const toolMp = $('#tool-mp');
const mpViewer = $('#mp-viewer'), mpMonitor = $('#mp-monitor'), mpOverlay = $('#mp-overlay'), mpStarsC = $('#mp-stars-c');
let mpMap = null, mpBuilding = null, mpSuppress = false, mpPreloadKey = '', mpPreloading = null;
const NOT_LOADING_MSG = 'Map imagery isn’t loading. The usual cause is opening this file inside the Claude app or another preview, which blocks map servers; download it and open it in Chrome or Edge on a computer. If you are already in a browser, check your internet connection or try another imagery source.';
function mpZoomOffset() { const { LW } = mpLogical(); return Math.log2(Math.max(1, mpViewer.clientWidth) / LW); }
function mpShowMsg(text, isErr = false) { const m = $('#mp-msg'); m.textContent = text || ''; m.hidden = !text; m.classList.toggle('err', !!isErr); }
async function mpBuildPreview() {
  const token = {}; mpBuilding = token;
  if (mpMap) { mpMap.remove(); mpMap = null; }
  mpPreloadKey = '';
  mpShowMsg('');
  try {
    const map = await mpCreateMap($('#mp-map'), window.devicePixelRatio || 1, { interactive: true });
    if (mpBuilding !== token) { map.remove(); return; }
    mpMap = map;
    const diag = { ok: 0, err: 0 };
    map.on('move', () => { if (!mpSuppress && mpPlayer.playing) mpPlayer.pause(); mpDrawOverlay(mpPlayer.t); });
    // The globe/flat blend updates during each render, so redraw markers after the map draws to keep them pinned.
    map.on('render', () => mpDrawOverlay(mpPlayer.t));
    map.on('data', ev => { if (ev.dataType === 'source' && ev.tile) { diag.ok++; if ($('#mp-msg').classList.contains('err') && diag.ok > 3) mpShowMsg(''); } });
    map.on('error', ev => {
      diag.err++;
      const st = ev?.error?.status;
      if (MAP_SOURCES[mp.source].provider === 'google' && (st === 403 || st === 401)) mpShowMsg(`Google refused the tiles (error ${st}). Check that Map Tiles API is enabled and billing is linked.`, true);
    });
    setTimeout(() => { if (mpMap === map && diag.ok === 0) mpShowMsg(NOT_LOADING_MSG, true); }, 9000);
    map.once('load', () => mpRender(mpPlayer.t));
    mpRender(mpPlayer.t);
  } catch (e) {
    const msg = e.code === 'nolib' ? 'The map engine didn’t load. If you opened this inside the Claude app or another preview, download the file and open it in Chrome or Edge on a computer. Otherwise check your internet connection and reload.'
      : /webgl/i.test(e.message || '') ? 'This browser couldn’t start 3D graphics (WebGL). Turn on hardware acceleration in your browser settings, or try Chrome.'
      : (e.message || String(e));
    mpShowMsg(msg, true);
  }
  googleCopy = '';
  if (MAP_SOURCES[mp.source].provider === 'google' && gKey) googleCopyright(mp.source).then(c => { googleCopy = c; mpDrawOverlay(mpPlayer.t); });
  updateTileBar();
}
function mpRender(t) {
  if (mpMap) {
    const cam = mpCamera(t);
    mpSuppress = true;
    mpMap.jumpTo({ center: cam.center, zoom: cam.zoom + mpZoomOffset(), bearing: cam.bearing, pitch: cam.pitch });
    mpSuppress = false;
  }
  mpDrawOverlay(t);
}
function mpDrawOverlay(t) {
  const dpr = window.devicePixelRatio || 1;
  const w = Math.round(mpViewer.clientWidth * dpr), h = Math.round(mpViewer.clientHeight * dpr);
  if (mpOverlay.width !== w || mpOverlay.height !== h) { mpOverlay.width = w; mpOverlay.height = h; }
  const ctx = mpOverlay.getContext('2d');
  ctx.clearRect(0, 0, w, h);
  const { LW } = mpLogical();
  const s = w / LW;
  const view = mpMap ? mapView(mpMap, dpr) : { project: () => null };
  mpOverlays(ctx, w, h, s, t, view, mpCreditText());
  if (mpPlayer && mpPlayer.guides) drawGuides(ctx, w, h, mp.out.aspect, s);
}
function mpDrawStarsBg() {
  const dpr = window.devicePixelRatio || 1;
  const w = Math.round(mpViewer.clientWidth * dpr), h = Math.round(mpViewer.clientHeight * dpr);
  mpStarsC.width = w; mpStarsC.height = h;
  const ctx = mpStarsC.getContext('2d');
  ctx.clearRect(0, 0, w, h);
  if (mp.stars && mp.projection === 'globe') { const { LW } = mpLogical(); drawStars(ctx, w, h, w / LW); }
}
function mpFit() {
  fitViewer(mpMonitor, mpViewer, mp.out.aspect);
  mpDrawStarsBg();
  if (mpMap) mpMap.resize();
  mpRender(mpPlayer.t);
}
new ResizeObserver(() => { if (!$('#tool-mp').hidden) mpFit(); }).observe(mpMonitor);

/* Load the imagery the camera will pass through, so the preview plays without gaps */
function mpCamKey() {
  return JSON.stringify([mp.mode, mp.source, mp.projection, mp.terrain, mp.buildings, mp.target, mp.endCenter, mp.startZoom, mp.endZoom, mp.spin, mp.rotate, mp.pitch, mp.duration, mp.delay, mp.ease, mp.rA, mp.rB, mp.rStops, mp.rPause, mp.rPath, mp.rBend, mp.rCam, mp.rFollowZoom, mp.rDraw, mp.rFlyIn, mp.areas.map(a => a.id), mp.out.aspect, Math.round(mpViewer.clientWidth)]);
}
async function mpBeforePlay() {
  if (!mpMap) return true;
  if (mpPreloading) { mpPreloading.cancel = true; return false; }
  const key = mpCamKey();
  if (key === mpPreloadKey) return true;
  const job = { cancel: false }; mpPreloading = job;
  const T = mpTotal(), N = clamp(Math.round(T * 4), 14, 60);
  for (let i = 0; i <= N && !job.cancel; i++) {
    mpShowMsg(`Loading imagery for the preview… ${Math.round(i / N * 100)}%. Press play again to start now.`);
    mpRender(T * i / N);
    await waitIdle(mpMap, 4000);
  }
  mpPreloading = null;
  mpShowMsg('');
  if (!job.cancel) mpPreloadKey = key;
  return true;
}

function mpRenderTrack() {
  const tr = $('#mp-track'), T = mpTotal();
  const blk = (from, len, c, label, dim) => `<div class="blk${dim ? ' dim' : ''}" style="left:${from / T * 100}%;width:${len / T * 100}%;--c:${c}">${label ? `<span>${label}</span>` : ''}</div>`;
  let html = '';
  if (mp.mode === 'route') {
    const r = routeTimes();
    if (mp.delay) html += blk(0, mp.delay, '#8a857c', 'Hold', true);
    if (r.fly) html += blk(mp.delay, r.fly, '#f4c443', 'Fly in');
    html += blk(r.drawStart, r.drawEnd - r.drawStart, mp.lineColor, midStops() ? `Line · ${midStops()} stop${midStops() === 1 ? '' : 's'}` : 'Line');
    if (mp.hold) html += blk(r.drawEnd, mp.hold, '#8a857c', 'Hold', true);
  } else if (mp.mode === 'area') {
    const a = areaTimes();
    if (mp.delay) html += blk(0, mp.delay, '#8a857c', 'Hold', true);
    html += blk(mp.delay, mp.duration, '#f4c443', 'Fly in');
    html += blk(a.drawStart, a.drawEnd - a.drawStart, mp.aColor, 'Outline');
    if (mp.hold) html += blk(a.drawEnd, mp.hold, '#8a857c', 'Hold', true);
  } else {
    if (mp.delay) html += blk(0, mp.delay, '#8a857c', 'Hold', true);
    html += blk(mp.delay, mp.duration, '#f4c443', 'Zoom');
    if (mp.hold) html += blk(mp.delay + mp.duration, mp.hold, '#8a857c', 'Hold', true);
  }
  const ms = mpMarkerStart();
  if (mp.mode !== 'area' && (mp.marker !== 'none' || mp.labelStyle !== 'off')) html += `<div class="blk" style="left:${ms / T * 100}%;width:${Math.max(0.2, T - ms) / T * 100}%;--c:${mp.markerColor};top:auto;bottom:2px;height:3px"></div>`;
  tr.innerHTML = html + '<div class="head"></div>';
}
function mpUpdateLocCard() {
  $('#mp-loc-name').textContent = mp.name || 'Unnamed place';
  $('#mp-loc-coords').textContent = fmtCoords(mp.target.lat, mp.target.lng);
  $('#mp-loc-via').innerHTML = '';
  $('#mp-loc-via').append(document.createTextNode((mp.via || '') + ' · '));
  const a = document.createElement('a');
  a.href = `https://www.google.com/maps/search/?api=1&query=${mp.target.lat.toFixed(6)},${mp.target.lng.toFixed(6)}`;
  a.target = '_blank'; a.rel = 'noopener'; a.textContent = 'Check in Google Maps';
  $('#mp-loc-via').appendChild(a);
  $('#mp-reset-view').hidden = !mp.endCenter;
  $('#mp-sample-tag').hidden = mp.via !== 'Sample location';
  const last = routeGeo ? routeGeo.stops.length - 1 : 0;
  $('#mp-route-name').textContent = routeGeo ? routeGeo.stops.map((s, k) => (k === 0 ? mp.rLabelA || 'Start' : k === last ? mp.rLabelB || 'Destination' : s.label || `Stop ${k}`)).join(' → ') : 'No route yet';
  $('#mp-route-dist').textContent = routeGeo ? `${fmtDist(routeGeo.distKm)}${routeGeo.isRoad ? ' by road' : ' straight-line'}` : '';
  $('#mp-route-sample').hidden = !mp.rSample;
  mpAreasUI();
}
/* The list of areas in the clip: color, label (editable), where the lines came from, and a remove button */
function mpAreasUI() {
  const host = $('#mp-alist');
  $('#mp-aempty').hidden = mp.areas.length > 0;
  $('#mp-aadd').disabled = !mp.areas.length || mp.areas.length >= 8;
  const sig = JSON.stringify(mp.areas.map((a, i) => [a.id, a.label, areaColor(i)]));
  if (host.dataset.sig === sig) return;
  host.dataset.sig = sig;
  host.innerHTML = '';
  mp.areas.forEach((A, i) => {
    const li = document.createElement('li');
    li.innerHTML = `<input type="color" aria-label="Color for area ${i + 1}"><input type="text" aria-label="Label for area ${i + 1}" maxlength="80"><button class="x" type="button" aria-label="Remove area ${i + 1}">×</button><span class="d"></span>`;
    const [col, txt] = li.querySelectorAll('input');
    col.value = areaColor(i).toLowerCase();
    col.addEventListener('input', () => { if (i === 0 && !A.color) bind.mp.aColor.set(col.value); else A.color = col.value; host.dataset.sig = ''; mpRefresh(); });
    txt.value = A.label;
    txt.addEventListener('input', () => { A.label = txt.value; host.dataset.sig = JSON.stringify(mp.areas.map((a, k) => [a.id, a.label, areaColor(k)])); if (mp.aTitleAuto) bind.mp.aTitle.set(autoTitle()); mpRefresh(); });
    li.querySelector('.d').textContent = [A.detail, A.via].filter(Boolean).join(' · ');
    li.querySelector('.x').addEventListener('click', () => { mp.areas.splice(i, 1); if (mp.aTitleAuto) bind.mp.aTitle.set(autoTitle()); mpRefresh(); mpRefreshGoogleCredit(); });
    host.appendChild(li);
  });
}
const autoTitle = () => (mp.areas.length === 1 ? mp.areas[0].label : mp.areas.map(a => a.short || a.label).join(' · '));
const ZOOM_NAMES = [[2.5, 'world'], [4.5, 'continent'], [6.5, 'country'], [9.5, 'region'], [12.5, 'city'], [15.5, 'neighborhood'], [18.5, 'street'], [99, 'building']];
const zoomName = z => ZOOM_NAMES.find(([lim]) => z < lim)[1];
function mpFields() {
  toolMp.dataset.mode = mp.mode;
  $('#mp-gkey-f').hidden = MAP_SOURCES[mp.source].provider !== 'google';
  $('#mp-exag-f').hidden = !mp.terrain;
  $('#mp-bcolor-f').hidden = !mp.buildings;
  $('#mp-bend-f').hidden = mp.rPath !== 'arc';
  $('#mp-rzoom-f').hidden = mp.rCam !== 'follow';
  const fly = mp.mode !== 'route' || mp.rFlyIn;
  $('#mp-aq').placeholder = { state: 'e.g. Texas, or TX', county: 'e.g. Travis County, TX', place: 'e.g. Austin, TX', cd: 'e.g. TX-35, or Texas 35th', sldu: 'e.g. TX SD 14, or an address', sldl: 'e.g. TX HD 49, or an address', radius: 'Center: a place or address' }[mp.aKind];
  $('#mp-arad-f').hidden = mp.aKind !== 'radius';
  $('#mp-aorder-f').hidden = mp.areas.length < 2;
  $('#mp-z0-f').hidden = !fly; $('#mp-spin-f').hidden = !fly; $('#mp-dur-f').hidden = !fly;
}
function mpRefresh() {
  mpFields();
  const { OW, OH } = dims(mp.out);
  $('#mp-fmt').textContent = `${OW} × ${OH} · ${mp.out.aspect} · ${mp.out.fps} fps`;
  $('#mp-z0-name').textContent = zoomName(mp.startZoom);
  $('#mp-z1-name').textContent = zoomName(mp.endZoom);
  $('#mp-rzoom-name').textContent = zoomName(mp.rFollowZoom);
  $('#mp-credit').textContent = `On-screen credit: ${mpCreditText()}. Check each provider’s terms before publishing commercially.`;
  $('#mp-outnote').textContent = 'Map renders wait for every tile, so they run slower than real time. 4K requests about four times as many tiles.';
  mpRenderTrack();
  mpUpdateLocCard();
  if (mpPlayer.t > mpTotal()) mpPlayer.seek(mpTotal()); else { mpRender(mpPlayer.t); mpPlayer.sync(); }
  saveSoon('mp', mp);
}

/* ----- location parsing ----- */
function parseCoords(s) {
  const dms = [...s.matchAll(/(\d{1,3})\s*°\s*(\d{1,2})\s*['′’]\s*(\d{1,2}(?:\.\d+)?)\s*(?:["″”]|'')?\s*([NSEW])/gi)];
  if (dms.length >= 2) {
    const v = dms.slice(0, 2).map(m => { let d = +m[1] + (+m[2]) / 60 + (+m[3]) / 3600; if (/[SW]/i.test(m[4])) d = -d; return { d, lat: /[NS]/i.test(m[4]) }; });
    const la = v.find(x => x.lat), lo = v.find(x => !x.lat);
    if (la && lo) return { lat: la.d, lng: lo.d };
  }
  const m = s.match(/^\s*\(?\s*(-?\d{1,2}(?:\.\d+)?)\s*°?\s*([NS])?\s*[,;\s]\s*(-?\d{1,3}(?:\.\d+)?)\s*°?\s*([EW])?\s*\)?\s*$/i);
  if (m) {
    let lat = +m[1], lng = +m[3];
    if (m[2] && /s/i.test(m[2])) lat = -Math.abs(lat);
    if (m[4] && /w/i.test(m[4])) lng = -Math.abs(lng);
    if (Math.abs(lat) <= 90 && Math.abs(lng) <= 180) return { lat, lng };
  }
  return null;
}
function parseLocationInput(raw) {
  const s = (raw || '').trim();
  if (!s) return { error: 'Paste a Google Maps link, coordinates, or an address.' };
  if (/^(https?:\/\/)?(maps\.app\.goo\.gl|goo\.gl\/maps|g\.co\/kgs)/i.test(s)) {
    return { error: 'Short share links (maps.app.goo.gl) can’t be opened from here. Open the link in your browser, then copy the full address from the address bar, or right-click the spot in Google Maps and copy its coordinates.' };
  }
  if (/^https?:\/\//i.test(s) || /google\.[a-z.]+\/maps/i.test(s)) {
    let u = null; try { u = new URL(/^https?:/i.test(s) ? s : 'https://' + s); } catch (e) { /* not a URL */ }
    let str = s; try { str = decodeURIComponent(s); } catch (e) { /* keep raw */ }
    let name = null, lat = null, lng = null, zoom = null;
    const place = str.match(/\/maps\/place\/([^/@?]+)/); if (place) name = place[1].replace(/\+/g, ' ').trim();
    const d = [...str.matchAll(/!3d(-?\d+(?:\.\d+)?)!4d(-?\d+(?:\.\d+)?)/g)].pop(); if (d) { lat = +d[1]; lng = +d[2]; }
    const at = str.match(/@(-?\d+(?:\.\d+)?),(-?\d+(?:\.\d+)?)(?:,(\d+(?:\.\d+)?)z)?/);
    if (at) { if (lat == null) { lat = +at[1]; lng = +at[2]; } if (at[3]) zoom = +at[3]; }
    if (u) for (const k of ['q', 'query', 'll', 'center', 'destination', 'daddr']) {
      const v = u.searchParams.get(k); if (!v) continue;
      const c = parseCoords(v);
      if (c && lat == null) { lat = c.lat; lng = c.lng; } else if (!c && !name) name = v;
    }
    const search = str.match(/\/maps\/search\/([^/@?]+)/); if (search && !name) name = search[1].replace(/\+/g, ' ');
    if (lat != null && Math.abs(lat) <= 90 && Math.abs(lng) <= 180) return { lat, lng, name, zoom, via: 'From your Google Maps link' };
    if (name) return { query: name, name, zoom };
    return { error: 'That link has no location in it. Open the place in Google Maps and copy the link again.' };
  }
  const c = parseCoords(s);
  if (c) return { ...c, name: null, via: 'From coordinates' };
  return { query: s };
}
/* Address search. Find asks Google (with a key) or Nominatim for up to five matches and keeps the
   others on screen so a wrong guess is one click to fix. Suggestions while typing come from Photon,
   which is built for search-as-you-type; Nominatim's usage policy forbids autocomplete. */
const PHOTON = 'https://photon.komoot.io/api/';
const ZOOM_BY_KIND = { house: 18, building: 18, street: 17, road: 17, locality: 15, neighbourhood: 15, suburb: 14, district: 14, village: 14, town: 13, city: 12, county: 9, state: 7, region: 7, country: 5 };
const picked = new Map();  // text put in an input by picking a match → that exact place, so lookups don't guess again
const joinParts = (name, parts) => parts.filter((p, k) => p && p !== name && parts.indexOf(p) === k).join(', ');
const dedupe = list => list.filter((a, k) => !list.slice(0, k).some(b => b.name === a.name && b.detail === a.detail));
function nominatimPlace(g) {
  const bb = (g.boundingbox || []).map(Number);
  const parts = g.display_name.split(',').map(x => x.trim());
  const name = g.name || parts[0];
  return { lat: +g.lat, lng: +g.lon, name, detail: joinParts(name, parts).split(', ').slice(0, 4).join(', '), kind: g.addresstype || g.type, bbox: bb.length === 4 ? [bb[0], bb[2], bb[1], bb[3]] : null, via: `OpenStreetMap search · ${g.display_name}` };
}
function photonPlace(f) {
  const p = f.properties || {}, [lng, lat] = f.geometry.coordinates;
  const street = [p.housenumber, p.street].filter(Boolean).join(' ');
  const name = p.name || street || p.city || p.county || p.state || p.country || fmtCoords(lat, lng);
  const detail = joinParts(name, [p.name ? street : '', p.district, p.city, p.state, p.country]);
  const ex = p.extent;  // [west, north, east, south]
  return { lat, lng, name, detail, kind: p.type || p.osm_value, bbox: ex && ex.length === 4 ? [ex[3], ex[0], ex[1], ex[2]] : null, via: `OpenStreetMap search · ${detail ? name + ', ' + detail : name}` };
}
async function geocode(q) {
  if (gKey) {
    try {
      const r = await fetch(`https://maps.googleapis.com/maps/api/geocode/json?address=${encodeURIComponent(q)}&key=${encodeURIComponent(gKey)}`);
      const j = await r.json();
      if (j.status === 'OK' && j.results.length) {
        return dedupe(j.results.slice(0, 5).map((g, k) => {
          const vp = g.geometry.viewport, parts = g.formatted_address.split(',').map(x => x.trim());
          const name = k === 0 ? q.split(',')[0].trim() : parts[0];
          return { lat: g.geometry.location.lat, lng: g.geometry.location.lng, name, detail: joinParts(name, parts), kind: g.types?.[0], bbox: vp ? [vp.southwest.lat, vp.southwest.lng, vp.northeast.lat, vp.northeast.lng] : null, via: `Google Geocoding · ${g.formatted_address}` };
        }));
      }
      if (j.status === 'ZERO_RESULTS') throw new Error(`No match for “${q}”. Add a city or country.`);
    } catch (e) { if (/No match/.test(e.message)) throw e; console.warn('Google geocoding unavailable, using OpenStreetMap', e); }
  }
  let r;
  try { r = await fetch(`https://nominatim.openstreetmap.org/search?format=jsonv2&limit=5&q=${encodeURIComponent(q)}`, { headers: { 'Accept-Language': navigator.language || 'en' } }); }
  catch (e) { throw new Error('Address search is unreachable right now. Paste coordinates instead (right-click a spot in Google Maps to copy them).'); }
  if (!r.ok) throw new Error(`Address search refused the request (error ${r.status}). Paste coordinates instead.`);
  const j = await r.json();
  if (!j.length) throw new Error(`No match for “${q}”. Check the spelling, add a city or country, or paste coordinates.`);
  return dedupe(j.map(nominatimPlace));
}
async function suggestPlaces(q, signal, near) {
  const lang = (navigator.language || 'en').slice(0, 2).toLowerCase();
  const params = new URLSearchParams({ q, limit: '6', lang: ['en', 'de', 'fr', 'it'].includes(lang) ? lang : 'en' });
  if (near) { params.set('lat', near.lat.toFixed(4)); params.set('lon', near.lng.toFixed(4)); params.set('zoom', '7'); params.set('location_bias_scale', '0.3'); }
  const r = await fetch(`${PHOTON}?${params}`, { signal });
  if (!r.ok) throw new Error(`Suggestions unavailable (${r.status})`);
  const j = await r.json();
  return dedupe((j.features || []).filter(f => f.geometry?.type === 'Point').map(photonPlace)).slice(0, 5);
}
async function resolveLocation(raw) {
  const hit = picked.get((raw || '').trim());
  if (hit) return { ...hit, matches: [hit] };
  const parsed = parseLocationInput(raw);
  if (parsed.error) throw new Error(parsed.error);
  if (!parsed.query) return { ...parsed, name: parsed.name || null };
  const matches = await geocode(parsed.query);
  return { ...matches[0], zoom: parsed.zoom, name: matches[0].name || parsed.name, matches };
}
/** A suggestion list under an address input: typing shows matches, ↑ ↓ and Enter pick one, Escape closes it.
    show(places, heading) also lists "Other matches" after a Find. */
function attachSuggest(input, list, { onPick, near = () => null }) {
  let timer = 0, ctl = null, items = [], active = -1, sticky = false;
  const cancel = () => { clearTimeout(timer); if (ctl) { ctl.abort(); ctl = null; } };
  const close = () => {
    items = []; active = -1; sticky = false; list.innerHTML = ''; list.hidden = true;
    input.setAttribute('aria-expanded', 'false'); input.removeAttribute('aria-activedescendant');
  };
  const mark = k => {
    active = k;
    $$('.sg-item', list).forEach((li, j) => li.setAttribute('aria-selected', String(j === k)));
    if (k >= 0) { input.setAttribute('aria-activedescendant', `${list.id}-${k}`); $(`#${list.id}-${k}`).scrollIntoView({ block: 'nearest' }); }
  };
  const choose = k => {
    const pl = items[k]; if (!pl) return;
    const text = pl.detail ? `${pl.name}, ${pl.detail}` : pl.name;
    input.value = text; picked.set(text, pl);
    cancel(); close(); onPick(pl);
  };
  const show = (places, heading = '') => {
    close();
    if (!places.length) return;
    items = places; sticky = !!heading;
    if (heading) list.insertAdjacentHTML('beforeend', '<li class="sg-head" role="presentation"></li>');
    if (heading) list.lastChild.textContent = heading;
    places.forEach((pl, k) => {
      const li = document.createElement('li');
      li.id = `${list.id}-${k}`; li.className = 'sg-item'; li.setAttribute('role', 'option'); li.setAttribute('aria-selected', 'false');
      li.innerHTML = '<strong></strong><span></span>';
      li.firstChild.textContent = pl.name; li.lastChild.textContent = pl.detail || '';
      li.addEventListener('pointerdown', e => e.preventDefault());  // keep focus in the input
      li.addEventListener('click', () => choose(k));
      list.appendChild(li);
    });
    // Suggestions float over the controls below, so they don't shift under the pointer; "Other matches" stays in the flow
    // (.field is a grid, so an absolute child would start at its top: place it under the input explicitly)
    list.classList.toggle('floating', !heading);
    list.style.top = heading ? '' : `${input.offsetTop + input.offsetHeight + 4}px`;
    list.hidden = false; input.setAttribute('aria-expanded', 'true');
  };
  input.addEventListener('input', () => {
    cancel();
    const q = input.value.trim();
    if (q.length < 3 || parseLocationInput(q).query !== q) { close(); return; }  // links and coordinates need no suggestions
    timer = setTimeout(async () => {
      const c = ctl = new AbortController();
      try {
        const places = await suggestPlaces(q, c.signal, near());
        if (c === ctl && input.value.trim() === q && document.activeElement === input) show(places);
      } catch (e) { if (e.name !== 'AbortError' && c === ctl) close(); }
    }, 350);
  });
  input.addEventListener('keydown', e => {
    const open = !list.hidden && items.length;
    if (open && e.key === 'ArrowDown') { e.preventDefault(); mark((active + 1) % items.length); }
    else if (open && e.key === 'ArrowUp') { e.preventDefault(); mark((active - 1 + items.length) % items.length); }
    else if (e.key === 'Escape') { cancel(); if (open) { e.preventDefault(); close(); } }  // also stops suggestions still on their way
    else if (e.key === 'Enter') {
      cancel();
      if (open && active >= 0) { e.preventDefault(); e.stopImmediatePropagation(); choose(active); } else close();
    }
  });
  input.addEventListener('blur', () => { cancel(); if (!sticky) close(); });
  return { show, close, cancel };
}
function zoomForBox([s, w, n, e]) {
  const { LW, LH } = mpLogical();
  const lngSpan = Math.max(1e-6, e - w);
  const yv = lat => Math.log(Math.tan(Math.PI / 4 + clamp(lat, -85, 85) * Math.PI / 360));
  const latSpan = Math.max(1e-9, yv(n) - yv(s));
  const zx = Math.log2(LW * 360 / (256 * lngSpan)), zy = Math.log2(LH * 2 * Math.PI / (256 * latSpan));
  return clamp(Math.min(zx, zy) - 0.4, 3, 18);
}
async function mpFind() {
  const st = $('#mp-status');
  mp.query = $('#mp-query').value;
  st.textContent = 'Looking it up…'; st.className = 'status';
  $('#mp-find').disabled = true;
  placeSuggest.close();
  try {
    const res = await resolveLocation(mp.query);
    mpApplyPlace(res);
    const others = (res.matches || []).slice(1);
    if (others.length) {
      placeSuggest.show(others, 'Other matches');
      st.textContent = 'Found it. Not the right place? Pick another match below.';
    }
  } catch (e) {
    st.textContent = e.message || String(e); st.className = 'status err';
  } finally { $('#mp-find').disabled = false; }
}
function mpApplyPlace(res) {
  const st = $('#mp-status');
  mp.target = { lat: res.lat, lng: res.lng };
  mp.endCenter = null;
  mp.name = res.name || fmtCoords(res.lat, res.lng);
  mp.via = res.via || 'Found';
  if (mp.labelAuto || !mp.label) bind.mp.label.set(res.name || '');
  const z = res.zoom ? clamp(res.zoom, 3, 20) : res.bbox ? zoomForBox(res.bbox) : ZOOM_BY_KIND[res.kind] ?? null;
  if (z != null) bind.mp.endZoom.set(Math.round(z * 10) / 10);
  st.textContent = 'Found it. Press play to preview the zoom.'; st.className = 'status ok';
  mpRefresh();
  mpPlayer.seek(mpTotal());
  mpRefreshGoogleCredit();
}
function mpRefreshGoogleCredit() {
  if (MAP_SOURCES[mp.source].provider === 'google' && gKey) googleCopyright(mp.source).then(c => { googleCopy = c; mpDrawOverlay(mpPlayer.t); });
}
async function fetchRoad(points) {
  let r;
  try { r = await fetch(`https://router.project-osrm.org/route/v1/driving/${points.map(p => `${p.lng},${p.lat}`).join(';')}?overview=full&geometries=geojson`); }
  catch (e) { throw new Error('The road-routing service is unreachable right now, so the route is drawn as an arc.'); }
  if (!r.ok) throw new Error(`The road-routing service returned an error (${r.status}), so the route is drawn as an arc.`);
  const j = await r.json();
  if (j.code !== 'Ok' || !j.routes?.length) throw new Error('No driving route connects these places, so the route is drawn as an arc.');
  let c = j.routes[0].geometry.coordinates;
  const stride = Math.ceil(c.length / 1500);
  if (stride > 1) c = c.filter((_, i) => i % stride === 0 || i === c.length - 1);
  return { coords: c.map(p => [+p[0].toFixed(6), +p[1].toFixed(6)]), dist: j.routes[0].distance };
}
async function ensureRoad() {
  if (mp.rPath !== 'roads' || routePoints().length < 2 || mp.rRoadKey === roadKey()) return;
  const st = $('#mp-route-status');
  st.textContent = 'Finding the driving route…'; st.className = 'status';
  try {
    const road = await fetchRoad(routePoints());
    mp.rRoad = road.coords; mp.rRoadDist = road.dist; mp.rRoadKey = roadKey();
    st.textContent = 'Driving route ready.'; st.className = 'status ok';
  } catch (e) {
    mp.rRoad = null; mp.rRoadKey = '';
    st.textContent = e.message; st.className = 'status err';
  }
  buildRoute(); mpRefresh();
}
async function mpBuildRouteFromInputs() {
  const st = $('#mp-route-status');
  mp.rQueryA = $('#mp-ra').value; mp.rQueryB = $('#mp-rb').value;
  mp.rStops = mp.rStops.filter(s => (s.q || '').trim());   // empty stop boxes are dropped
  mpStopsUI();
  const queries = [mp.rQueryA, ...mp.rStops.map(s => s.q), mp.rQueryB];
  st.textContent = queries.length > 2 ? `Looking up ${queries.length} places…` : 'Looking up both places…'; st.className = 'status';
  $('#mp-route-go').disabled = true;
  try {
    const found = [];
    for (const [k, q] of queries.entries()) {
      if (k && found[k - 1].via?.startsWith('OpenStreetMap')) await sleep(1100); // be polite to the free search service
      try { found.push(await resolveLocation(q)); }
      catch (e) { throw new Error(k === 0 ? `From: ${e.message}` : k === queries.length - 1 ? `To: ${e.message}` : `Stop ${k}: ${e.message}`); }
    }
    const a = found[0], b = found[found.length - 1];
    const nameOf = (res, q) => res.name || q.split(',')[0].trim();
    mp.rA = { lat: a.lat, lng: a.lng }; mp.rB = { lat: b.lat, lng: b.lng }; mp.rSample = false;
    mp.rStops.forEach((s, k) => { const res = found[k + 1]; Object.assign(s, { lat: res.lat, lng: res.lng, label: nameOf(res, s.q) }); });
    if (mp.rLabelAuto) {
      const na = nameOf(a, mp.rQueryA), nb = nameOf(b, mp.rQueryB);
      bind.mp.rLabelA.set(na); bind.mp.rLabelB.set(nb); bind.mp.rTitle.set([na, ...mp.rStops.map(s => s.label), nb].join(' → '));
    }
    mp.rRoadKey = '';
    st.textContent = 'Route ready. Press play to preview.'; st.className = 'status ok';
    buildRoute(); mpRefresh();
    await ensureRoad();
    mpPlayer.seek(mpTotal());
    mpRefreshGoogleCredit();
  } catch (e) {
    st.textContent = e.message || String(e); st.className = 'status err';
  } finally { $('#mp-route-go').disabled = false; }
}
// Attached before the Enter handlers below so Enter on a highlighted suggestion picks it instead of searching
const placeSuggest = attachSuggest($('#mp-query'), $('#mp-query-list'), {
  near: () => (mp.via === 'Sample location' ? null : mp.target),
  onPick: pl => { mp.query = $('#mp-query').value; mpApplyPlace(pl); },
});
attachSuggest($('#mp-ra'), $('#mp-ra-list'), { onPick: () => {} });
attachSuggest($('#mp-rb'), $('#mp-rb-list'), { near: () => mp.rA, onPick: () => {} });
/* Stops between From and To. Each box keeps its text in mp.rStops[i].q; Build route looks them all up. */
const MAX_STOPS = 8;
function mpStopsUI() {
  const host = $('#mp-rstops');
  host.innerHTML = '';
  mp.rStops.forEach((stop, i) => {
    const row = document.createElement('div');
    row.className = 'stop-row';
    row.innerHTML = `<label class="lbl" for="mp-rs-${i}">Stop ${i + 1}</label>
      <div class="row-inline"><input type="text" id="mp-rs-${i}" placeholder="Link, coordinates, or address" autocomplete="off" role="combobox" aria-autocomplete="list" aria-expanded="false" aria-controls="mp-rs-${i}-list">
      <button class="btn small" type="button" aria-label="Remove stop ${i + 1}">×</button></div>
      <ul class="suggest" id="mp-rs-${i}-list" role="listbox" aria-label="Matching places for stop ${i + 1}" hidden></ul>`;
    const input = row.querySelector('input');
    input.value = stop.q || '';
    input.addEventListener('input', () => { stop.q = input.value; delete stop.lat; delete stop.lng; saveSoon('mp', mp); });
    attachSuggest(input, row.querySelector('ul'), { near: () => mp.rA, onPick: () => { stop.q = input.value; } });
    input.addEventListener('keydown', e => { if (e.key === 'Enter') { e.preventDefault(); mpBuildRouteFromInputs(); } });
    row.querySelector('button').addEventListener('click', () => { mp.rStops.splice(i, 1); mpStopsUI(); mp.rRoadKey = ''; buildRoute(); mpRefresh(); });
    host.appendChild(row);
  });
  $('#mp-rstop-add').disabled = mp.rStops.length >= MAX_STOPS;
}
$('#mp-rstop-add').addEventListener('click', () => {
  if (mp.rStops.length >= MAX_STOPS) return;
  mp.rStops.push({ q: '' });
  mpStopsUI();
  $(`#mp-rs-${mp.rStops.length - 1}`).focus();
});
mpStopsUI();
/* Outline mode: find the area by name, district, or an address inside it */
/* "Outline" replaces the clip's areas with this one; "Add" puts it after the others (up to 8) */
async function mpFindArea(add = false) {
  const st = $('#mp-astatus');
  mp.aQuery = $('#mp-aq').value;
  st.textContent = 'Looking it up…'; st.className = 'status';
  $('#mp-ago').disabled = true; $('#mp-aadd').disabled = true;
  try {
    const A = await CS.boundaries.find(mp.aKind, mp.aQuery, { locate: resolveLocation, radius: mp.aRadius, unit: mp.aUnit });
    if (add && mp.areas.some(a => a.id === A.id)) throw new Error(`${A.label} is already in this clip.`);
    mp.areas = add ? [...mp.areas, A].slice(0, 8) : [A];
    if (mp.aTitleAuto || !mp.aTitle) bind.mp.aTitle.set(autoTitle());
    st.textContent = add ? `Added ${A.label}. ${mp.areas.length} areas in this clip.` : `Found ${A.label}. Press play to preview.`; st.className = 'status ok';
    mpRefresh();
    mpPlayer.seek(mpTotal());
    mpRefreshGoogleCredit();
  } catch (e) {
    st.textContent = e.message || String(e); st.className = 'status err';
  } finally { $('#mp-ago').disabled = false; mpAreasUI(); }
}
$('#mp-ago').addEventListener('click', () => mpFindArea(false));
$('#mp-aadd').addEventListener('click', () => mpFindArea(true));
$('#mp-aq').addEventListener('keydown', e => { if (e.key === 'Enter') { e.preventDefault(); mpFindArea(e.shiftKey && mp.areas.length > 0); } });
$('#mp-find').addEventListener('click', mpFind);
$('#mp-query').addEventListener('keydown', e => { if (e.key === 'Enter') { e.preventDefault(); mpFind(); } });
$('#mp-route-go').addEventListener('click', mpBuildRouteFromInputs);
['#mp-ra', '#mp-rb'].forEach(id => $(id).addEventListener('keydown', e => { if (e.key === 'Enter') { e.preventDefault(); mpBuildRouteFromInputs(); } }));
$('#mp-pin-center').addEventListener('click', () => {
  if (!mpMap) return;
  const c = mpMap.getCenter();
  mp.target = { lat: c.lat, lng: c.lng }; mp.via = 'Pin placed by hand';
  mpRefresh();
  toast('Pin moved to the center of the preview.');
});
$('#mp-use-view').addEventListener('click', () => {
  if (!mpMap) return;
  const c = mpMap.getCenter();
  mp.endCenter = { lat: c.lat, lng: c.lng };
  bind.mp.endZoom.set(Math.round(clamp(mpMap.getZoom() - mpZoomOffset() + 1, 3, 21) * 10) / 10);
  bind.mp.rotate.set(Math.round(mpMap.getBearing()));
  bind.mp.pitch.set(Math.round(mpMap.getPitch()));
  mpRefresh();
  toast('End shot saved. The zoom now lands on this framing.');
});
$('#mp-reset-view').addEventListener('click', () => { mp.endCenter = null; mpRefresh(); });

/* key */
$('#mp-gkey').value = gKey;
$('#mp-gkey-test').addEventListener('click', async () => {
  const st = $('#mp-gkey-status');
  gKey = $('#mp-gkey').value.trim(); store.set('gkey', gKey);
  for (const k of Object.keys(gSessions)) delete gSessions[k];
  store.set('gsessions', gSessions);
  if (!gKey) { st.textContent = 'Key removed.'; st.className = 'status'; return; }
  st.textContent = 'Testing with Google…'; st.className = 'status';
  try {
    await googleSession(MAP_SOURCES[mp.source].provider === 'google' ? mp.source : 'google-sat');
    st.textContent = 'Key works. Google tiles are ready.'; st.className = 'status ok';
    if (MAP_SOURCES[mp.source].provider === 'google') mpBuildPreview();
  } catch (e) { st.textContent = e.message; st.className = 'status err'; }
});
$('#mp-help-link').addEventListener('click', e => { e.preventDefault(); $('#help-dlg').showModal(); });

const REBUILD_KEYS = new Set(['source', 'projection', 'terrain', 'buildings']);
let exagTimer = 0;
const mpChanged = key => {
  if (key === 'label') mp.labelAuto = false;
  if (key === 'rLabelA' || key === 'rLabelB' || key === 'rTitle') mp.rLabelAuto = false;
  if (key === 'aTitle') mp.aTitleAuto = false;
  if (key === 'aColor') $('#mp-alist').dataset.sig = '';
  if (REBUILD_KEYS.has(key)) { mpBuildPreview(); mpDrawStarsBg(); }
  if (key === 'exaggeration' && mpMap) { clearTimeout(exagTimer); exagTimer = setTimeout(() => { try { if (mpMap.getTerrain()) mpMap.setTerrain({ source: 'dem', exaggeration: mp.exaggeration }); } catch (e) { /* ignore */ } }, 120); }
  if (key === 'stars') mpDrawStarsBg();
  if (key === 'buildingColor' && mpMap) { try { mpMap.getStyle().layers.filter(l => l.type === 'fill-extrusion').forEach(l => mpMap.setPaintProperty(l.id, 'fill-extrusion-color', mp.buildingColor)); } catch (e) { /* style not ready */ } }
  if (key === 'out.aspect') mpFit();
  if (key === 'startZoom' && mp.startZoom > mp.endZoom) bind.mp.endZoom.set(mp.startZoom);
  if (key === 'endZoom' && mp.endZoom < mp.startZoom) bind.mp.startZoom.set(mp.endZoom);
  if (key === 'rPath' || key === 'rBend') { buildRoute(); if (key === 'rPath') ensureRoad(); }
  if (key === 'mode') { mpPlayer.pause(); mpRefresh(); mpPlayer.seek(mpTotal()); mpRefreshGoogleCredit(); return; }
  mpRefresh();
};
$('#mp-query').value = mp.query;
$('#mp-aq').value = mp.aQuery;
$('#mp-ra').value = mp.rQueryA; $('#mp-rb').value = mp.rQueryB;
bindSeg('mp-mode', mp, 'mode', mpChanged);
bindSelect('mp-source', mp, 'source', mpChanged);
bindSeg('mp-proj', mp, 'projection', mpChanged);
bindCheck('mp-stars', mp, 'stars', mpChanged);
bindCheck('mp-terrain', mp, 'terrain', mpChanged);
bindRange('mp-exag', mp, 'exaggeration', v => (+v).toFixed(1) + '×', mpChanged);
bindCheck('mp-buildings', mp, 'buildings', mpChanged);
bindColor('mp-bcolor', mp, 'buildingColor', mpChanged);
bindCheck('mp-flyin', mp, 'rFlyIn', mpChanged);
bind.mp.startZoom = bindRange('mp-z0', mp, 'startZoom', v => (+v).toFixed(1), mpChanged);
bind.mp.endZoom = bindRange('mp-z1', mp, 'endZoom', v => (+v).toFixed(1), mpChanged);
bindRange('mp-spin', mp, 'spin', v => v + '°', mpChanged);
bindSeg('mp-rcam', mp, 'rCam', mpChanged);
bindRange('mp-rzoom', mp, 'rFollowZoom', v => (+v).toFixed(1), mpChanged);
bind.mp.rotate = bindRange('mp-rotate', mp, 'rotate', v => v + '°', mpChanged);
bind.mp.pitch = bindRange('mp-pitch', mp, 'pitch', v => v + '°', mpChanged);
bindRange('mp-dur', mp, 'duration', fmtSec, mpChanged);
bindRange('mp-rdraw', mp, 'rDraw', fmtSec, mpChanged);
bindRange('mp-rpause', mp, 'rPause', fmtSec, mpChanged);
bindRange('mp-delay', mp, 'delay', fmtSec, mpChanged);
bindRange('mp-hold', mp, 'hold', fmtSec, mpChanged);
bindSeg('mp-ease', mp, 'ease', mpChanged);
bindSeg('mp-rpath', mp, 'rPath', mpChanged);
bindRange('mp-bend', mp, 'rBend', v => Math.round(v * 100) + '%', mpChanged);
bindColor('mp-lcolor', mp, 'lineColor', mpChanged);
bindRange('mp-lwidth', mp, 'lineWidth', v => v + 'px', mpChanged);
bindCheck('mp-lglow', mp, 'lineGlow', mpChanged);
bindCheck('mp-ldash', mp, 'lineDash', mpChanged);
bindSeg('mp-ricon', mp, 'rIcon', mpChanged);
bindSeg('mp-marker', mp, 'marker', mpChanged);
bindColor('mp-mcolor', mp, 'markerColor', mpChanged);
bind.mp.label = bindText('mp-label', mp, 'label', mpChanged);
bind.mp.aTitle = bindText('mp-alabel', mp, 'aTitle', mpChanged);
bindSelect('mp-akind', mp, 'aKind', mpChanged);
bind.mp.aColor = bindColor('mp-acolor', mp, 'aColor', mpChanged);
bindRange('mp-arad', mp, 'aRadius', v => String(v), mpChanged);
bindSeg('mp-aunit', mp, 'aUnit', mpChanged);
bindSeg('mp-aorder', mp, 'aOrder', mpChanged);
bindRange('mp-awidth', mp, 'aWidth', v => v + 'px', mpChanged);
bindRange('mp-afill', mp, 'aFill', v => Math.round(v * 100) + '%', mpChanged);
bindRange('mp-adim', mp, 'aDim', v => Math.round(v * 100) + '%', mpChanged);
bindRange('mp-adraw', mp, 'aDraw', fmtSec, mpChanged);
bind.mp.rLabelA = bindText('mp-rla', mp, 'rLabelA', mpChanged);
bind.mp.rLabelB = bindText('mp-rlb', mp, 'rLabelB', mpChanged);
bind.mp.rTitle = bindText('mp-rtitle', mp, 'rTitle', mpChanged);
bindSeg('mp-lstyle', mp, 'labelStyle', mpChanged);
bindColor('mp-lbg', mp, 'labelBg', mpChanged);
bindColor('mp-lfg', mp, 'labelFg', mpChanged);
bindCheck('mp-coords', mp, 'coords', mpChanged);
bindCheck('mp-vignette', mp, 'vignette', mpChanged);

const mpPlayer = makePlayer('mp', { total: mpTotal, fps: () => mp.out.fps, render: mpRender, track: $('#mp-track'), beforePlay: mpBeforePlay });

/* Map render: a full-size map draws each frame; we repaint until a frame has been drawn with every tile in view,
   then composite that frame. (MapLibre's 'idle' event can lag in globe mode, so we check after each render.) */
function waitIdle(map, timeout = 20000) {
  return new Promise(res => {
    const start = performance.now();
    const check = () => {
      if (map.areTilesLoaded() || performance.now() - start > timeout) { map.off('render', check); res(); }
      else map.triggerRepaint();
    };
    map.on('render', check);
    map.triggerRepaint();
  });
}
async function mpRenderOne() {
  if (!window.maplibregl) { toast('The map engine didn’t load. Open this file in Chrome or Edge on a computer.'); return 'error'; }
  mpPlayer.pause();
  if (mp.mode === 'route' && !routeGeo) { toast('Build a route first.'); return 'error'; }
  if (mp.mode === 'area' && !mp.areas.length) { toast('Outline an area first.'); return 'error'; }
  const { q, OW, OH } = dims(mp.out);
  const R = 2 * q, { LW, LH } = mpLogical();
  let map = null, credit = mpCreditText(), last = '';
  const startTiles = tileCount.n;
  const label = mp.mode === 'route' ? (mp.rTitle || 'route') : mp.mode === 'area' ? (mp.areas.length === 1 ? mp.areas[0].short : mp.aTitle || mp.areas[0].short) : (mp.label || mp.name);
  return runRender({
    title: `Rendering ${mp.out.aspect} map`, OW, OH, fps: mp.out.fps, total: mpTotal(), format: mp.out.format,
    name: `${{ route: 'route', area: 'outline' }[mp.mode] || 'zoom'}_${slug(label)}_${OW}x${OH}`,
    async setup({ stage, host }) {
      const box = document.createElement('div');
      box.style.cssText = `width:${LW}px;height:${LH}px;position:absolute;left:0;top:0;`;
      host.appendChild(box);
      host.style.transform = `scale(${stage.clientWidth / LW})`;
      map = await mpCreateMap(box, R, { preserve: true });
      await new Promise((res, rej) => { map.once('load', res); map.once('error', ev => { if (!map.loaded()) rej(new Error('The map couldn’t load. ' + (ev?.error?.message || ''))); }); setTimeout(res, 30000); });
      if (MAP_SOURCES[mp.source].provider === 'google') { googleCopy = await googleCopyright(mp.source); credit = mpCreditText(); }
    },
    async frame(ctx, t) {
      const cam = mpCamera(t);
      const key = [cam.center[0].toFixed(9), cam.center[1].toFixed(9), cam.zoom.toFixed(6), cam.bearing.toFixed(4), cam.pitch.toFixed(4)].join();
      if (key !== last) { map.jumpTo(cam); await waitIdle(map); last = key; }
      ctx.setTransform(1, 0, 0, 1, 0, 0);
      ctx.fillStyle = '#03050b'; ctx.fillRect(0, 0, OW, OH);
      if (mp.stars && mp.projection === 'globe') drawStars(ctx, OW, OH, R);
      ctx.drawImage(map.getCanvas(), 0, 0, OW, OH);
      mpOverlays(ctx, OW, OH, R, t, mapView(map, R), credit);
      const used = tileCount.n - startTiles;
      if (used) $('#render-eta').textContent = `${used.toLocaleString()} Google tiles`;
    },
    teardown() { if (map) map.remove(); store.set('tilecount', tileCount); },
  });
}
async function mpStill() {
  if (!window.maplibregl) { toast('The map engine didn’t load. Open this file in Chrome or Edge on a computer.'); return; }
  const { q, OW, OH } = dims(mp.out);
  const R = 2 * q, { LW, LH } = mpLogical();
  const host = document.createElement('div');
  host.style.cssText = `position:fixed;left:0;top:0;width:${LW}px;height:${LH}px;opacity:0;pointer-events:none;z-index:-1;`;
  document.body.appendChild(host);
  toast('Rendering a full-size frame…');
  let map;
  try {
    map = await mpCreateMap(host, R, { preserve: true });
    await new Promise(res => { map.once('load', res); setTimeout(res, 30000); });
    map.jumpTo(mpCamera(mpPlayer.t)); await waitIdle(map);
    const c = document.createElement('canvas'); c.width = OW; c.height = OH;
    const ctx = c.getContext('2d');
    ctx.fillStyle = '#03050b'; ctx.fillRect(0, 0, OW, OH);
    if (mp.stars && mp.projection === 'globe') drawStars(ctx, OW, OH, R);
    ctx.drawImage(map.getCanvas(), 0, 0, OW, OH);
    mpOverlays(ctx, OW, OH, R, mpPlayer.t, mapView(map, R), mpCreditText());
    await saveStill(c, `map_frame_${slug(mp.mode === 'route' ? mp.rTitle : mp.mode === 'area' ? mp.areas[0]?.short || 'area' : mp.label || mp.name)}_${OW}x${OH}.png`);
  } catch (e) { toast(e.message || 'Couldn’t render the frame.'); }
  finally { if (map) map.remove(); host.remove(); }
}
buildOutputPanel('mp', mp.out, mpChanged, {
  render: mpRenderOne,
  renderBoth: () => renderBoth(mp.out, () => { mpFit(); mpRefresh(); }, mpRenderOne),
  still: mpStill,
});
buildPresetPanel('mp', mp, MP_PRESET_KEYS, MP_BUILTINS, () => { buildRoute(); ensureRoad(); mpBuildPreview(); mpDrawStarsBg(); mpFit(); mpRefresh(); });

let mpStarted = false;
CS.tools.mp = {
  id: 'mp',
  player: mpPlayer,
  boot() {
    buildRoute();
    mpRefresh();
    mpPlayer.seek(mpTotal());
  },
  show() {
    requestAnimationFrame(() => {
      mpFit();
      if (!mpStarted) { mpStarted = true; mpBuildPreview(); }
    });
  },
  hide() { mpPlayer.pause(); },
  debug: { mp, mpPlayer, mpFindArea, areaTimes, get mpMap() { return mpMap; }, get routeGeo() { return routeGeo; }, routeTimes, routeProgress, stopTimes },
};
})();
