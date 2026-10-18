/* Clip Studio · US boundaries for the Map Zoom outline mode
   (states, counties, cities, congressional and state legislative districts, and radius circles)
   Classic script (not an ES module) so index.html still works when opened straight from disk.
   Puts CS.boundaries = { find(kind, query, { locate }) } on window.CS; map.js does the drawing.

   Boundaries come from the US Census Bureau's TIGERweb service (current vintage, exact lines, and a "which area holds
   this point" query). If TIGERweb can't be reached, states and counties fall back to the us-atlas files on jsDelivr
   (Census cartographic boundaries, simplified to 1:10 million); districts have no fallback.
   A found area is { kind, label, short, detail, credit, rings: [[[lng, lat], …], …], bbox: [s, w, n, e], center, labelAt }. */
(() => {
'use strict';
const TIGER = 'https://tigerweb.geo.census.gov/arcgis/rest/services/TIGERweb/tigerWMS_Current/MapServer';
const ATLAS = window.CS.lib('us-atlas', 'https://cdn.jsdelivr.net/npm/us-atlas@3.0.1/counties-10m.json');
const MAX_POINTS = 6000;

// FIPS code, postal code, name
const STATES = [
  ['01', 'AL', 'Alabama'], ['02', 'AK', 'Alaska'], ['04', 'AZ', 'Arizona'], ['05', 'AR', 'Arkansas'], ['06', 'CA', 'California'],
  ['08', 'CO', 'Colorado'], ['09', 'CT', 'Connecticut'], ['10', 'DE', 'Delaware'], ['11', 'DC', 'District of Columbia'], ['12', 'FL', 'Florida'],
  ['13', 'GA', 'Georgia'], ['15', 'HI', 'Hawaii'], ['16', 'ID', 'Idaho'], ['17', 'IL', 'Illinois'], ['18', 'IN', 'Indiana'],
  ['19', 'IA', 'Iowa'], ['20', 'KS', 'Kansas'], ['21', 'KY', 'Kentucky'], ['22', 'LA', 'Louisiana'], ['23', 'ME', 'Maine'],
  ['24', 'MD', 'Maryland'], ['25', 'MA', 'Massachusetts'], ['26', 'MI', 'Michigan'], ['27', 'MN', 'Minnesota'], ['28', 'MS', 'Mississippi'],
  ['29', 'MO', 'Missouri'], ['30', 'MT', 'Montana'], ['31', 'NE', 'Nebraska'], ['32', 'NV', 'Nevada'], ['33', 'NH', 'New Hampshire'],
  ['34', 'NJ', 'New Jersey'], ['35', 'NM', 'New Mexico'], ['36', 'NY', 'New York'], ['37', 'NC', 'North Carolina'], ['38', 'ND', 'North Dakota'],
  ['39', 'OH', 'Ohio'], ['40', 'OK', 'Oklahoma'], ['41', 'OR', 'Oregon'], ['42', 'PA', 'Pennsylvania'], ['44', 'RI', 'Rhode Island'],
  ['45', 'SC', 'South Carolina'], ['46', 'SD', 'South Dakota'], ['47', 'TN', 'Tennessee'], ['48', 'TX', 'Texas'], ['49', 'UT', 'Utah'],
  ['50', 'VT', 'Vermont'], ['51', 'VA', 'Virginia'], ['53', 'WA', 'Washington'], ['54', 'WV', 'West Virginia'], ['55', 'WI', 'Wisconsin'],
  ['56', 'WY', 'Wyoming'], ['60', 'AS', 'American Samoa'], ['66', 'GU', 'Guam'], ['69', 'MP', 'Northern Mariana Islands'], ['72', 'PR', 'Puerto Rico'],
  ['78', 'VI', 'U.S. Virgin Islands'],
].map(([fips, abbr, name]) => ({ fips, abbr, name }));
const byFips = Object.fromEntries(STATES.map(s => [s.fips, s]));
const norm = s => s.toLowerCase().replace(/[.']/g, '').replace(/\s+/g, ' ').trim();
const BY_LENGTH = STATES.slice().sort((a, b) => b.name.length - a.name.length);   // "West Virginia" before "Virginia"
const abbrState = c => STATES.find(x => x.abbr === c.toUpperCase()) || null;
/** A state named in the text: a full name, an uppercase postal code ("TX"), or the whole text being a code ("tx").
    Lowercase two-letter words ("in", "or", "me") are left alone so addresses don't turn into states. */
function stateIn(text) {
  const t = ` ${norm(text).replace(/[^a-z0-9 ]/g, ' ')} `;
  const named = BY_LENGTH.find(s => t.includes(` ${norm(s.name)} `));
  if (named) return named;
  if (/^\s*[a-z]{2}\s*$/i.test(text)) return abbrState(text.trim());
  for (const c of text.match(/\b[A-Z]{2}\b/g) || []) { const s = abbrState(c); if (s) return s; }
  return null;
}
const ordinal = n => { const v = n % 100; return n + (v >= 11 && v <= 13 ? 'th' : ({ 1: 'st', 2: 'nd', 3: 'rd' })[n % 10] || 'th'); };
const sqlText = s => `'${s.replace(/'/g, "''").toUpperCase()}'`;

/* ---------- geometry ---------- */
function ringsOf(geom) {
  if (!geom) return [];
  if (geom.type === 'Polygon') return geom.coordinates;
  if (geom.type === 'MultiPolygon') return geom.coordinates.flat();
  return [];
}
function rdp(pts, eps) {   // Ramer–Douglas–Peucker, iterative
  if (pts.length < 5) return pts;
  const keep = new Uint8Array(pts.length); keep[0] = keep[pts.length - 1] = 1;
  const stack = [[0, pts.length - 1]];
  while (stack.length) {
    const [a, b] = stack.pop();
    const [ax, ay] = pts[a], [bx, by] = pts[b], dx = bx - ax, dy = by - ay, L = Math.hypot(dx, dy) || 1e-12;
    let best = -1, bi = -1;
    for (let i = a + 1; i < b; i++) {
      const d = Math.abs(dy * (pts[i][0] - ax) - dx * (pts[i][1] - ay)) / L;
      if (d > best) { best = d; bi = i; }
    }
    if (best > eps) { keep[bi] = 1; stack.push([a, bi], [bi, b]); }
  }
  return pts.filter((_, i) => keep[i]);
}
/** Keeps an area under MAX_POINTS so the preview stays quick; tiny islands go first */
function simplify(rings) {
  let total = rings.reduce((n, r) => n + r.length, 0), eps = 0.0001, out = rings;
  while (total > MAX_POINTS && eps < 1) {
    out = rings.map(r => rdp(r, eps)).filter(r => r.length >= 4);
    if (!out.length) out = [rdp(rings.slice().sort((a, b) => b.length - a.length)[0], eps)];
    total = out.reduce((n, r) => n + r.length, 0);
    eps *= 2;
  }
  return out.map(r => r.map(([x, y]) => [+x.toFixed(5), +y.toFixed(5)]));
}
const ringArea = r => { let a = 0; for (let i = 0, j = r.length - 1; i < r.length; j = i++) a += (r[j][0] - r[i][0]) * (r[j][1] + r[i][1]); return a / 2; };
function centroid(r) {
  let a = 0, cx = 0, cy = 0;
  for (let i = 0, j = r.length - 1; i < r.length; j = i++) {
    const f = r[j][0] * r[i][1] - r[i][0] * r[j][1];
    a += f; cx += (r[j][0] + r[i][0]) * f; cy += (r[j][1] + r[i][1]) * f;
  }
  return Math.abs(a) < 1e-12 ? { lng: r[0][0], lat: r[0][1] } : { lng: cx / (3 * a), lat: cy / (3 * a) };
}
/** Bounds as [s, w, n, e]. Areas that cross the 180° line (Alaska's Aleutians) are measured with longitudes in 0–360. */
function boundsOf(rings) {
  const span = shift => {
    let w = Infinity, e = -Infinity, s = Infinity, n = -Infinity;
    for (const r of rings) for (const [x0, y] of r) { const x = shift && x0 < 0 ? x0 + 360 : x0; w = Math.min(w, x); e = Math.max(e, x); s = Math.min(s, y); n = Math.max(n, y); }
    return [s, w, n, e];
  };
  const a = span(false), b = span(true);
  return b[3] - b[1] < a[3] - a[1] - 1 ? b : a;   // only when clearly narrower: most areas measure the same either way
}
function pointIn(rings, lng, lat) {
  let inside = false;
  for (const r of rings) for (let i = 0, j = r.length - 1; i < r.length; j = i++) {
    const [xi, yi] = r[i], [xj, yj] = r[j];
    if ((yi > lat) !== (yj > lat) && lng < (xj - xi) * (lat - yi) / (yj - yi) + xi) inside = !inside;
  }
  return inside;
}
function makeArea(meta, rawRings) {
  const rings = simplify(rawRings);
  const bbox = boundsOf(rings);
  const mid = (bbox[1] + bbox[3]) / 2, cLng = mid > 180 ? mid - 360 : mid;
  // Longitudes within 180° of the center, so a line crossing the 180° meridian stays continuous on a flat map
  const near = x => { while (x - cLng > 180) x -= 360; while (x - cLng < -180) x += 360; return x; };
  const local = rings.map(r => r.map(([x, y]) => [+near(x).toFixed(5), y]));
  const lab = centroid(local.slice().sort((a, b) => Math.abs(ringArea(b)) - Math.abs(ringArea(a)))[0]);
  return { ...meta, rings: local, bbox, center: { lat: (bbox[0] + bbox[2]) / 2, lng: cLng }, labelAt: lab };
}

/* ---------- TIGERweb ---------- */
async function getJSON(url, ms = 20000) {
  const ctl = new AbortController(), tm = setTimeout(() => ctl.abort(), ms);
  try {
    const r = await fetch(url, { signal: ctl.signal });
    if (!r.ok) throw new Error(`HTTP ${r.status}`);
    const j = await r.json();
    if (j.error) throw new Error(j.error.message || 'service error');
    return j;
  } finally { clearTimeout(tm); }
}
let layersP = null;
/** Layer ids are looked up by name, so a new Congress or a reshuffled service doesn't break the tool */
function tigerLayers() {
  if (!layersP) {
    layersP = getJSON(`${TIGER}?f=json`).then(j => {
      const L = j.layers || [];
      const one = re => L.find(l => re.test(l.name))?.id;
      const latest = re => L.map(l => ({ id: l.id, m: l.name.match(re) })).filter(x => x.m).sort((a, b) => +(b.m[1] || 0) - +(a.m[1] || 0))[0];
      const cd = latest(/^(\d+)(?:st|nd|rd|th) Congressional Districts$/i);
      const up = latest(/^(\d{4})?\s*State Legislative Districts\s*-\s*Upper$/i), low = latest(/^(\d{4})?\s*State Legislative Districts\s*-\s*Lower$/i);
      return {
        state: one(/^States$/i), county: one(/^Counties$/i), place: one(/^Incorporated Places$/i),
        cd: cd?.id, congress: cd ? +cd.m[1] : null,
        sldu: up?.id, sldl: low?.id, sldYear: up?.m[1] || low?.m[1] || null,
      };
    }).catch(e => { layersP = null; throw e; });
  }
  return layersP;
}
async function tigerQuery(layer, where, point) {
  const p = new URLSearchParams({ outFields: '*', returnGeometry: 'true', outSR: '4326', f: 'geojson', maxAllowableOffset: '0.0003', geometryPrecision: '5' });
  if (point) Object.entries({ geometry: `${point.lng},${point.lat}`, geometryType: 'esriGeometryPoint', inSR: '4326', spatialRel: 'esriSpatialRelIntersects', where: '1=1' }).forEach(([k, v]) => p.set(k, v));
  else p.set('where', where);
  const j = await getJSON(`${TIGER}/${layer}/query?${p}`);
  return (j.features || []).filter(f => f.geometry);
}

/* ---------- us-atlas fallback ---------- */
let atlasP = null;
function atlas() {
  if (!atlasP) {
    atlasP = getJSON(ATLAS, 30000).then(topo => {
      const [kx, ky] = topo.transform.scale, [dx, dy] = topo.transform.translate;
      const arcs = topo.arcs.map(arc => { let x = 0, y = 0; return arc.map(([a, b]) => [(x += a) * kx + dx, (y += b) * ky + dy]); });
      const ring = idx => { const out = []; idx.forEach((i, k) => { const a = i >= 0 ? arcs[i] : arcs[~i].slice().reverse(); out.push(...(k ? a.slice(1) : a)); }); return out; };
      const feats = obj => obj.geometries.map(g => ({ id: g.id, name: g.properties?.name || '', rings: g.type === 'Polygon' ? g.arcs.map(ring) : g.type === 'MultiPolygon' ? g.arcs.flatMap(p => p.map(ring)) : [] }));
      return { states: feats(topo.objects.states), counties: feats(topo.objects.counties) };
    }).catch(e => { atlasP = null; throw e; });
  }
  return atlasP;
}
const ATLAS_CREDIT = 'Boundaries: US Census Bureau (us-atlas)';
const ATLAS_DETAIL = 'Census cartographic boundaries, simplified · TIGERweb was unreachable';

/* ---------- finding an area ---------- */
const COUNTY_SUFFIX = /\s+(county|parish|borough|census area|city and borough|municipality|municipio)$/i;
function stateArea(f, src) {
  const st = byFips[src === 'tiger' ? f.properties.STATE : f.id] || { name: f.properties?.NAME || f.name, abbr: '' };
  return { kind: 'state', label: st.name, short: st.abbr || st.name, detail: src === 'tiger' ? 'State · US Census Bureau TIGERweb' : ATLAS_DETAIL, credit: src === 'tiger' ? 'Boundaries: US Census Bureau' : ATLAS_CREDIT, id: src === 'tiger' ? f.properties.GEOID : f.id };
}
function countyArea(f, src) {
  const fips = src === 'tiger' ? f.properties.STATE : f.id.slice(0, 2), st = byFips[fips];
  const name = src === 'tiger' ? f.properties.NAME : `${f.name}${/city$/i.test(f.name) ? '' : fips === '22' ? ' Parish' : fips === '02' ? '' : ' County'}`;
  return { kind: 'county', label: `${name}, ${st ? st.name : ''}`.replace(/, $/, ''), short: `${name}, ${st ? st.abbr : ''}`.replace(/, $/, ''), detail: src === 'tiger' ? 'County · US Census Bureau TIGERweb' : ATLAS_DETAIL, credit: src === 'tiger' ? 'Boundaries: US Census Bureau' : ATLAS_CREDIT, id: src === 'tiger' ? f.properties.GEOID : f.id };
}
function districtArea(f, congress) {
  const geoid = String(f.properties.GEOID || ''), st = byFips[geoid.slice(0, 2)] || { name: '', abbr: '' };
  const num = geoid.slice(2), n = parseInt(num, 10);
  const atLarge = num === '00' || num === '98';
  const label = `${st.name}’s ${atLarge ? 'At-Large' : ordinal(n)} District`;
  return { kind: 'cd', label, short: `${st.abbr}-${atLarge ? 'AL' : n}`, detail: `${congress ? ordinal(congress) + ' Congress' : 'Congressional district'} · US Census Bureau TIGERweb`, credit: 'Boundaries: US Census Bureau', id: geoid };
}
/** District names in the forms people write them → { st, num } (num 0 = at-large). Anything else is treated as an address.
    "TX-35", "tx 35", "WY-AL", "Texas 35", "Texas’s 35th district", "35th district of Texas", "Wyoming at-large" */
function parseDistrict(q) {
  const s = q.trim().replace(/[’']s\b/gi, '').replace(/\s+/g, ' ');
  const DIST = '(?:\\s+(?:congressional\\s+)?district)?';
  const NUM = '(\\d{1,2})(?:st|nd|rd|th)?';
  const AL = 'at[- ]?large|al';
  let m = s.match(/^([A-Za-z]{2})\s*[-–\s]\s*(\d{1,2}|AL)$/i);
  if (m && abbrState(m[1])) return { st: abbrState(m[1]), num: /al/i.test(m[2]) ? 0 : +m[2] };
  for (const st of BY_LENGTH) {
    const name = st.name.replace(/\./g, '\\.');
    const forms = [
      [new RegExp(`^${name}\\s+${NUM}${DIST}$`, 'i'), 1],
      [new RegExp(`^${NUM}${DIST},?\\s+(?:of\\s+)?${name}$`, 'i'), 1],
      [new RegExp(`^${name}\\s+(?:${AL})${DIST}$`, 'i'), 0],
    ];
    for (const [re, hasNum] of forms) { const mm = s.match(re); if (mm) return { st, num: hasNum ? +mm[1] : 0 }; }
  }
  return null;
}
/** "TX SD 14", "TX-14", "Texas Senate 14", "Texas House District 49", "District 49, Texas" → { st, num } */
function parseLeg(q) {
  const s = q.trim().replace(/[’']s\b/gi, '').replace(/\s+/g, ' ');
  let m = s.match(/^([A-Za-z]{2})[\s-]*(?:SD|HD|AD|LD|S|H)?[\s-]*(\d{1,3})$/i);
  if (m && abbrState(m[1])) return { st: abbrState(m[1]), num: +m[1 + 1] };
  const CH = '(?:(?:state\\s+)?(?:senate|house|assembly|legislative|legislature)(?:\\s+of\\s+(?:representatives|delegates))?\\s+)?';
  for (const st of BY_LENGTH) {
    const name = st.name.replace(/\./g, '\\.');
    for (const re of [new RegExp(`^${name}\\s+${CH}(?:district\\s+)?(\\d{1,3})$`, 'i'), new RegExp(`^${CH}district\\s+(\\d{1,3}),?\\s+(?:of\\s+)?${name}$`, 'i')]) {
      const mm = s.match(re); if (mm) return { st, num: +mm[1] };
    }
  }
  return null;
}
function legArea(f, kind, year) {
  const p = f.properties, st = byFips[p.STATE] || { name: '', abbr: '' };
  const num = String(p.BASENAME || String(p.GEOID || '').slice(2).replace(/^0+/, ''));
  const name = String(p.NAME || `District ${num}`).replace(/^State\s+/i, '');
  return { kind, label: `${st.name} ${name}`.trim(), short: `${st.abbr} ${kind === 'sldu' ? 'SD' : 'HD'}-${num}`, detail: `${year ? year + ' ' : ''}state legislative districts · US Census Bureau TIGERweb`, credit: 'Boundaries: US Census Bureau', id: p.GEOID };
}
function placeArea(f) {
  const p = f.properties, st = byFips[p.STATE] || { name: '', abbr: '' };
  const kindWord = (String(p.NAME || '').match(/\b(city|town|village|borough|municipality|CDP)$/i) || [])[1] || 'city';
  return { kind: 'place', label: `${p.BASENAME || p.NAME}, ${st.name}`, short: `${p.BASENAME || p.NAME}, ${st.abbr}`, detail: `${kindWord[0].toUpperCase()}${kindWord.slice(1)} limits · US Census Bureau TIGERweb`, credit: 'Boundaries: US Census Bureau', id: p.GEOID };
}
/** A true-distance circle (great-circle radius) as a ring of n points */
function circleRing(lat, lng, km, n = 180) {
  const R = 6371.0088, d = km / R, r = Math.PI / 180, p1 = lat * r, l1 = lng * r, out = [];
  for (let i = 0; i <= n; i++) {
    const th = 2 * Math.PI * (i % n) / n;
    const p2 = Math.asin(Math.sin(p1) * Math.cos(d) + Math.cos(p1) * Math.sin(d) * Math.cos(th));
    const l2 = l1 + Math.atan2(Math.sin(th) * Math.sin(d) * Math.cos(p1), Math.cos(d) - Math.sin(p1) * Math.sin(p2));
    out.push([l2 / r, p2 / r]);
  }
  return out;
}
const trimNum = v => String(+(+v).toFixed(2));
async function find(kind, raw, { locate, radius = 10, unit = 'mi' } = {}) {
  const q = (raw || '').trim();
  if (!q) throw new Error(kind === 'radius' ? 'Type the place or address at the center of the circle.' : 'Type a name, like Texas, Travis County, TX, or TX-35, or an address inside the area.');
  if (kind === 'radius') {
    const r = +radius;
    if (!(r > 0) || r > 2000) throw new Error('Set a radius between 0.1 and 2,000.');
    let loc;
    try { loc = await locate(q); } catch (e) { throw new Error(`The center couldn’t be found. ${e.message}`); }
    const km = unit === 'km' ? r : r * 1.609344;
    const words = unit === 'km' ? 'km' : r === 1 ? 'mile' : 'miles';
    const where = (loc.name || q.split(',')[0]).trim();
    return { ...makeArea({ kind: 'radius', label: `${trimNum(r)} ${words} around ${where}`, short: `${trimNum(r)} ${unit} · ${where}`, detail: `Radius around ${where}`, credit: '', id: `r:${loc.lat.toFixed(5)},${loc.lng.toFixed(5)},${km}` }, [circleRing(loc.lat, loc.lng, km)]), mark: { lat: loc.lat, lng: loc.lng } };
  }
  let layers = null, tigerErr = null;
  try { layers = await tigerLayers(); } catch (e) { tigerErr = e; }
  const viaPoint = async () => {
    let loc;
    try { loc = await locate(q); } catch (e) { throw new Error(`No ${kind === 'cd' ? 'district' : kind} named “${q}”, and it couldn’t be found as an address either. ${e.message}`); }
    return { lat: loc.lat, lng: loc.lng };
  };

  if (kind === 'cd') {
    if (!layers || layers.cd == null) throw new Error('District lines come from the US Census Bureau, which couldn’t be reached. Check your connection and try again. States and counties still work.');
    const d = parseDistrict(q);
    if (d) {
      const tries = d.num <= 1 ? [String(d.num).padStart(2, '0'), '00', '98'] : [String(d.num).padStart(2, '0')];
      for (const num of [...new Set(tries)]) {
        const fs = await tigerQuery(layers.cd, `GEOID='${d.st.fips}${num}'`);
        if (fs.length) return makeArea(districtArea(fs[0], layers.congress), ringsOf(fs[0].geometry));
      }
      throw new Error(`${d.st.name} has no district ${d.num} in the ${ordinal(layers.congress)} Congress. Check the number, or type an address in the district.`);
    }
    const pt = await viaPoint();
    const fs = await tigerQuery(layers.cd, null, pt);
    if (!fs.length) throw new Error('That address isn’t inside a US congressional district.');
    return { ...makeArea(districtArea(fs[0], layers.congress), ringsOf(fs[0].geometry)), via: 'Found from the address' };
  }

  if (kind === 'sldu' || kind === 'sldl') {
    const layer = layers?.[kind], house = kind === 'sldu' ? 'State Senate' : 'State House';
    if (!layers || layer == null) throw new Error(`${house} district lines come from the US Census Bureau, which couldn’t be reached. Check your connection and try again.`);
    const d = parseLeg(q);
    if (d) {
      let fs = await tigerQuery(layer, `GEOID='${d.st.fips}${String(d.num).padStart(3, '0')}'`);
      if (!fs.length) fs = await tigerQuery(layer, `STATE='${d.st.fips}' AND BASENAME='${d.num}'`);
      if (fs.length) return makeArea(legArea(fs[0], kind, layers.sldYear), ringsOf(fs[0].geometry));
      throw new Error(`No ${house.toLowerCase()} district ${d.num} found in ${d.st.name}. Some states number or name districts differently; an address inside the district always works.`);
    }
    const fs = await tigerQuery(layer, null, await viaPoint());
    if (!fs.length) throw new Error(`That address isn’t inside a ${house.toLowerCase()} district${kind === 'sldl' ? ' (Nebraska has no state house)' : ''}.`);
    return { ...makeArea(legArea(fs[0], kind, layers.sldYear), ringsOf(fs[0].geometry)), via: 'Found from the address' };
  }

  if (kind === 'place') {
    if (!layers || layers.place == null) throw new Error('City limits come from the US Census Bureau, which couldn’t be reached. Check your connection and try again.');
    const comma = q.lastIndexOf(',');
    const st = comma > 0 ? stateIn(q.slice(comma + 1)) : null;
    const base = (comma > 0 && st ? q.slice(0, comma) : q).trim().replace(/\s+(city|town|village)$/i, '');
    if (!/\d/.test(base) && base.split(/\s+/).length <= 5) {
      const fs = await tigerQuery(layers.place, `UPPER(BASENAME)=${sqlText(base)}${st ? ` AND STATE='${st.fips}'` : ''}`);
      if (fs.length > 1 && !st) {
        const where = [...new Set(fs.map(f => byFips[f.properties.STATE]?.abbr).filter(Boolean))];
        throw new Error(`There’s a ${base} in ${where.length} states (${where.slice(0, 6).join(', ')}${where.length > 6 ? ', …' : ''}). Add the state, like “${base}, ${where[0]}”.`);
      }
      if (fs.length) return makeArea(placeArea(fs[0]), ringsOf(fs[0].geometry));
    }
    const fs = await tigerQuery(layers.place, null, await viaPoint());
    if (!fs.length) throw new Error('That address isn’t inside an incorporated city or town. Try County instead.');
    return { ...makeArea(placeArea(fs[0]), ringsOf(fs[0].geometry)), via: 'Found from the address' };
  }

  if (kind === 'state') {
    const st = q.length <= 3 || STATES.some(s => norm(s.name) === norm(q)) ? stateIn(q) : null;
    if (layers) {
      if (st) { const fs = await tigerQuery(layers.state, `STATE='${st.fips}'`); if (fs.length) return makeArea(stateArea(fs[0], 'tiger'), ringsOf(fs[0].geometry)); }
      const fs = await tigerQuery(layers.state, null, await viaPoint());
      if (!fs.length) throw new Error('That place isn’t in a US state.');
      return { ...makeArea(stateArea(fs[0], 'tiger'), ringsOf(fs[0].geometry)), via: 'Found from the address' };
    }
    const A = await atlas().catch(() => { throw new Error(`State lines couldn’t be loaded (${tigerErr.message}). Check your connection and try again.`); });
    let f = st && A.states.find(x => x.id === st.fips);
    if (!f) { const pt = await viaPoint(); f = A.states.find(x => pointIn(x.rings, pt.lng, pt.lat)); }
    if (!f) throw new Error('That place isn’t in a US state.');
    return makeArea(stateArea(f, 'atlas'), f.rings);
  }

  // County: "Travis County, TX", "Travis, Texas", "Orleans Parish", or an address
  const comma = q.lastIndexOf(',');
  const st = comma > 0 ? stateIn(q.slice(comma + 1)) : null;
  const namePart = (comma > 0 && st ? q.slice(0, comma) : q).trim();
  const looksLikeName = !/\d/.test(namePart) && namePart.split(/\s+/).length <= 5;
  const base = namePart.replace(COUNTY_SUFFIX, '').trim();
  if (layers) {
    if (looksLikeName) {
      const stClause = st ? ` AND STATE='${st.fips}'` : '';
      let fs = COUNTY_SUFFIX.test(namePart) || /\bcity$/i.test(namePart) ? await tigerQuery(layers.county, `UPPER(NAME)=${sqlText(namePart)}${stClause}`) : [];
      if (!fs.length) fs = await tigerQuery(layers.county, `UPPER(BASENAME)=${sqlText(base)}${stClause}`);
      if (fs.length > 1 && !st) {
        const where = [...new Set(fs.map(f => byFips[f.properties.STATE]?.abbr).filter(Boolean))];
        throw new Error(`There’s a ${fs[0].properties.NAME} in ${where.length} states (${where.slice(0, 6).join(', ')}${where.length > 6 ? ', …' : ''}). Add the state, like “${fs[0].properties.NAME}, ${where[0]}”.`);
      }
      if (fs.length) return makeArea(countyArea(fs[0], 'tiger'), ringsOf(fs[0].geometry));
    }
    const fs = await tigerQuery(layers.county, null, await viaPoint());
    if (!fs.length) throw new Error('That place isn’t in a US county.');
    return { ...makeArea(countyArea(fs[0], 'tiger'), ringsOf(fs[0].geometry)), via: 'Found from the address' };
  }
  const A = await atlas().catch(() => { throw new Error(`County lines couldn’t be loaded (${tigerErr.message}). Check your connection and try again.`); });
  let matches = looksLikeName ? A.counties.filter(c => norm(c.name) === norm(base) && (!st || c.id.startsWith(st.fips))) : [];
  if (matches.length > 1 && !st) {
    const where = [...new Set(matches.map(c => byFips[c.id.slice(0, 2)]?.abbr).filter(Boolean))];
    throw new Error(`There’s a ${base} County in ${where.length} states (${where.slice(0, 6).join(', ')}${where.length > 6 ? ', …' : ''}). Add the state, like “${base} County, ${where[0]}”.`);
  }
  let f = matches[0];
  if (!f) { const pt = await viaPoint(); f = A.counties.find(c => pointIn(c.rings, pt.lng, pt.lat)); }
  if (!f) throw new Error('That place isn’t in a US county.');
  return makeArea(countyArea(f, 'atlas'), f.rings);
}

window.CS = Object.assign(window.CS || {}, { boundaries: { find, parseDistrict, parseLeg, stateIn, bounds: boundsOf } });
})();
