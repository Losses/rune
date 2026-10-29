// The city, shared by the passes that draw the street, in linear HDR.
//
// The city is a grid. Across, column 0 is the street (|x| < HALF), columns ±1 the row of shops on
// either side (COL deep) and ±2 onwards the blocks behind. Along the street every row is UNIT
// long and every BLOCK-th row is a cross street. A ray walks the grid cell by cell (a 2D DDA)
// and stops at the first building: its wall or its roof. Over the street hang signs facing the
// camera, strings of lanterns or a tangle of wires on poles, traffic lights at the crossings,
// and people walk the street. Signs, shops and lanterns light the street, which is wet.
// Some blocks are busy (more and bigger signs, more people), some quiet; each has one big sign.
// The colours are those of the lights that make them: warm and cool white shops, red, yellow
// and white signs, a sky the city stains brown.
//
// The grid's sizes and what stands where along the street are in state.wgsl, as update works out
// once a frame what the rows ahead hold.

// ---- Colour ----

fn hsv(h: f32, s: f32, v: f32) -> vec3f {
  let p = abs(fract(vec3f(h) + vec3f(1.0, 2.0 / 3.0, 1.0 / 3.0)) * 6.0 - 3.0);
  return v * mix(vec3f(1.0), clamp(p - 1.0, vec3f(0.0), vec3f(1.0)), s);
}

// Sign colours as a real street has them: mostly red, yellow and white, some orange, now and
// then green or blue, rarely pink. Each district has a favourite it uses more.
fn palette(i: f32) -> vec3f {
  if (i < 1.0) {
    return vec3f(1.0, 0.07, 0.05);
  }
  if (i < 2.0) {
    return vec3f(1.0, 0.68, 0.08);
  }
  if (i < 3.0) {
    return vec3f(1.0, 0.84, 0.6);
  }
  if (i < 4.0) {
    return vec3f(0.84, 0.93, 1.0);
  }
  if (i < 5.0) {
    return vec3f(1.0, 0.33, 0.06);
  }
  if (i < 6.0) {
    return vec3f(0.1, 0.78, 0.3);
  }
  if (i < 7.0) {
    return vec3f(0.12, 0.33, 1.0);
  }
  return vec3f(1.0, 0.15, 0.42);
}

fn neon(k: f32) -> vec3f {
  let r = fract(k);
  var i: f32;
  if (fract(r * 7.31) < 0.3) {
    // The district's favourite: red, yellow, warm white or orange.
    i = select(select(select(4.0, 2.0, state.district < 0.75), 1.0, state.district < 0.5), 0.0, state.district < 0.25);
  } else if (r < 0.3) {
    i = 0.0;
  } else if (r < 0.46) {
    i = 1.0;
  } else if (r < 0.62) {
    i = 2.0;
  } else if (r < 0.72) {
    i = 3.0;
  } else if (r < 0.82) {
    i = 4.0;
  } else if (r < 0.9) {
    i = 5.0;
  } else if (r < 0.97) {
    i = 6.0;
  } else {
    i = 7.0;
  }
  let c = palette(i);
  return mix(vec3f(rv_luma(c)), c, params.sign_saturation);
}

// Which pattern sign `id` shows: `changes` goes up by one for each hit that is heard (less in a
// fade, none in silence), and a sign takes a new pattern each time it has gone up by two
// (`switch_share`, a half), at a phase of its own.
fn pattern_of(id: vec2f) -> f32 {
  return floor(state.changes * params.switch_share + h21(id + 0.5) * 5.3);
}

// Quiet sections leave some signs dark; each section picks its own.
fn lit(id: vec2f) -> f32 {
  return step(state.dark, h21(id * 3.71 + vec2f(input.section * 1.37, 2.9)));
}

// How bright the signs and the big screens are: 0.85 at a usual energy, dimmer in a quiet passage
// and brighter in a loud one (`bright`, the energy's 1 s average), swelling and dipping with the
// bass round that (`bass_swell`, 0 at a band_low of 1).
fn glow_now() -> f32 {
  return 0.85 * input.bright * (1.0 + input.bass_swell);
}

// ---- Layout ----

// The column x is in: 0 the street, ±1 the shops, ±2 and on the blocks behind.
fn column(x: f32) -> i32 {
  if (abs(x) < HALF) {
    return 0;
  }
  let j = i32(floor((abs(x) - HALF) / COL)) + 1;
  return select(-j, j, x > 0.0);
}

// Where column i starts and ends across.
fn column_span(i: i32) -> vec2f {
  if (i == 0) {
    return vec2f(-HALF, HALF);
  }
  let a = HALF + f32(abs(i) - 1) * COL;
  return select(vec2f(-a - COL, -a), vec2f(a, a + COL), i > 0);
}

// The box of the building in column i, row n across (x0, x1) and along (z0, z1). Shop buildings
// may stand back from the street; behind them alleys run between the buildings.
fn cell_box(i: i32, n: f32) -> vec4f {
  let span = column_span(i);
  if (abs(i) == 1) {
    let w = wall_x(n, f32(i));
    return select(vec4f(span.x, -w, n * UNIT, n * UNIT + UNIT), vec4f(w, span.y, n * UNIT, n * UNIT + UNIT), i > 0);
  }
  var z0 = n * UNIT;
  var z1 = z0 + UNIT;
  if (fract(n * 0.5) < 0.25) {
    z0 += 0.6;
  } else {
    z1 -= 0.6;
  }
  return vec4f(span.x + 0.6, span.y - 0.6, z0, z1);
}

// The billboard on the roof of shop building n (side s): its bottom and top, 0 for none.
fn billboard_of(n: f32, s: f32) -> vec2f {
  let b = first_row(n, s);
  let hb = height_of(i32(s), n);
  let on = h21(vec2f(b, s) + 33.3) < 0.35 && hb > 3.0 * STOREY;
  return select(vec2f(0.0), vec2f(hb, hb + 2.5 + 2.0 * h21(vec2f(b, s) + 3.3)), on);
}

// Now and then a building has a small shrine's torii on its roof, as Japanese city roofs do: its
// gate facing along the street, so it is seen square on going down it, towards the roof's street
// side so it shows over the edge. Where it stands (x, z of
// its middle), its size (1 is 3 m tall) and the roof's height; size 0 for none. Not on a roof
// with a billboard. The same for every cell of a building.
fn torii_of(i: i32, n: f32) -> vec4f {
  let hb = height_of(i, n);
  if (hb <= 0.0) {
    return vec4f(0.0);
  }
  var x0: f32;
  var x1: f32;
  var z0: f32;
  var z1: f32;
  var id: vec2f;
  if (abs(i) == 1) {
    let s = f32(i);
    if (billboard_of(n, s).y > 0.0) {
      return vec4f(0.0);
    }
    let b = first_row(n, s);
    let w = wall_x(n, s);
    let span = column_span(i);
    x0 = select(span.x, w, s > 0.0);
    x1 = select(-w, span.y, s > 0.0);
    z0 = b * UNIT;
    z1 = z0 + rows_of(n, s) * UNIT;
    id = vec2f(b, s);
  } else {
    // Behind the shops a building is two rows long.
    let box = cell_box(i, n);
    let k = floor(n * 0.5);
    x0 = box.x;
    x1 = box.y;
    z0 = k * 2.0 * UNIT + 0.6;
    z1 = z0 + 2.0 * UNIT - 1.2;
    id = vec2f(k, f32(i) * 7.3);
  }
  if (h21(id + 71.3) >= params.roof_torii) {
    return vec4f(0.0);
  }
  let size = min(0.85 + 0.45 * h21(id + 72.1), (x1 - x0) / 3.6);
  // Towards the street side of the roof, so it shows over the edge from the street.
  let street_side = select(x0, x1, f32(i) < 0.0);
  let x = mix(street_side, 0.5 * (x0 + x1), 0.45);
  return vec4f(x, mix(z0 + 1.0, z1 - 1.0, h21(id + 73.7)), size, hb);
}

// The torii over the street (street_torii): the roof torii's gate, scaled (its pillars 2.2 m
// either side of the middle, its kasagi reaching 3.5 m and topping 6.6 m, under the arch's
// 7.15 m), in the middle of its row.
const STREET_TORII: f32 = 2.2;

// The ray against roof torii r (from torii_of): t and the face (as box_hit), and the piece (0 a
// pillar, 1 the nuki, 2 the kasagi); t < 0 for a miss. Straight boxes: pillars, the nuki through
// them, the kasagi over it overhanging both sides.
fn torii_hit(o: vec3f, inv: vec3f, r: vec4f) -> vec3f {
  let base = vec3f(r.x, r.w, r.y);
  let k = r.z;
  var best = vec3f(-1.0, 0.0, 0.0);
  for (var piece = 0; piece < 4; piece++) {
    var c: vec3f;
    var h: vec3f;
    if (piece < 2) {
      c = vec3f(select(-1.0, 1.0, piece == 1), 1.4, 0.0);
      h = vec3f(0.12, 1.4, 0.12);
    } else if (piece == 2) {
      c = vec3f(0.0, 2.25, 0.0);
      h = vec3f(1.25, 0.08, 0.07);
    } else {
      c = vec3f(0.0, 2.88, 0.0);
      h = vec3f(1.6, 0.13, 0.17);
    }
    let q = box_hit(o, inv, base + (c - h) * k, base + (c + h) * k);
    if (q.x > 0.0 && (best.x < 0.0 || q.x < best.x)) {
      best = vec3f(q, f32(max(piece - 1, 0)));
    }
  }
  return best;
}

// Where the ray enters the box lo..hi (t > 0) and through which face (1 across, 2 along,
// 3 top or bottom), or t < 0.
fn box_hit(o: vec3f, inv: vec3f, lo: vec3f, hi: vec3f) -> vec2f {
  let t0 = (lo - o) * inv;
  let t1 = (hi - o) * inv;
  let tn = min(t0, t1);
  let tf = max(t0, t1);
  let near = max(max(tn.x, tn.y), tn.z);
  let far = min(min(tf.x, tf.y), tf.z);
  if (near > far || near < 1e-4) {
    return vec2f(-1.0, 0.0);
  }
  return vec2f(near, select(select(2.0, 3.0, near == tn.y), 1.0, near == tn.x));
}

fn inverse(d: vec3f) -> vec3f {
  return 1.0 / select(d, vec3f(1e-6), abs(d) < vec3f(1e-6));
}

// Whether the ray passes through the box lo..hi at all (from where it starts, inside or out).
fn in_box(o: vec3f, inv: vec3f, lo: vec3f, hi: vec3f) -> bool {
  let t0 = (lo - o) * inv;
  let t1 = (hi - o) * inv;
  let tn = min(t0, t1);
  let tf = max(t0, t1);
  return max(max(max(tn.x, tn.y), tn.z), 0.0) <= min(min(tf.x, tf.y), tf.z);
}

// Where the ray from o along rd (normalised) meets the capsule from a to b of radius r (iq's), or
// -1.
fn capsule_t(o: vec3f, rd: vec3f, a: vec3f, b: vec3f, r: f32) -> f32 {
  let ba = b - a;
  let oa = o - a;
  let baba = dot(ba, ba);
  let bard = dot(ba, rd);
  let baoa = dot(ba, oa);
  let rdoa = dot(rd, oa);
  let qa = baba - bard * bard;
  let qb = baba * rdoa - baoa * bard;
  let qc = baba * dot(oa, oa) - baoa * baoa - r * r * baba;
  let h = qb * qb - qa * qc;
  if (h < 0.0) {
    return -1.0;
  }
  let t = (-qb - sqrt(h)) / qa;
  let y = baoa + t * bard;
  if (y > 0.0 && y < baba) {
    return t;
  }
  // The rounded ends.
  let oc = select(o - b, oa, y <= 0.0);
  let b2 = dot(rd, oc);
  let h2 = b2 * b2 - dot(oc, oc) + r * r;
  return select(-1.0, -b2 - sqrt(h2), h2 > 0.0);
}

// Where the ray from o along rd (normalised) meets the sphere at c of radius r, or -1.
fn sphere_t(o: vec3f, rd: vec3f, c: vec3f, r: f32) -> f32 {
  let oc = o - c;
  let b = dot(oc, rd);
  let h = b * b - dot(oc, oc) + r * r;
  return select(-1.0, -b - sqrt(h), h > 0.0);
}

// Where the ray from o along d meets the ellipsoid at c with radii rad (t in d's units), or -1.
fn ellipsoid_t(o: vec3f, d: vec3f, c: vec3f, rad: vec3f) -> f32 {
  let oc = (o - c) / rad;
  let dd = d / rad;
  let a = dot(dd, dd);
  let b = dot(oc, dd);
  let h = b * b - a * (dot(oc, oc) - 1.0);
  return select(-1.0, (-b - sqrt(h)) / a, h > 0.0);
}

// The nearer of two hits (-1 for none).
fn nearer(a: f32, b: f32) -> f32 {
  return select(a, b, b > 0.0 && (a <= 0.0 || b < a));
}

// A sign's light, steady (as it lights the street) and with the beat (as it is seen).
fn sign_colour(n: f32, s: f32) -> vec3f {
  let id = vec2f(n, s);
  let strength = select(1.2, 2.2, h21(id + 6.1) > 0.4);
  return neon(h21(id + 4.7)) * strength * lit(id);
}

fn sign_light(n: f32, s: f32) -> vec3f {
  return sign_colour(n, s) * glow_now();
}


// What the shop of building n on side s gives off (black for a shutter or a cross street),
// following the energy and the bass as the signs do (as it lights the street and as it is seen).
fn shop_light(n: f32, s: f32) -> vec3f {
  return shop_colour(n, s) * 0.8 * input.bright * (1.0 + input.bass_swell);
}

fn shop_colour(n: f32, s: f32) -> vec3f {
  if (crossing(n)) {
    return vec3f(0.0);
  }
  let id = vec2f(n, s);
  let kind = shop_kind(n, s);
  var c = vec3f(0.0);
  if (kind == KONBINI) {
    c = vec3f(0.85, 0.95, 1.0) * 1.1;
  } else if (kind == IZAKAYA) {
    c = vec3f(1.0, 0.5, 0.2) * 0.8;
  } else if (kind == GAMES) {
    c = mix(neon(h21(id + 5.5)), vec3f(1.0), 0.5) * 1.2;
  } else if (kind == BOUTIQUE) {
    c = vec3f(1.0, 0.85, 0.7) * 0.3;
  } else if (kind == DRUGSTORE) {
    c = vec3f(1.0, 0.95, 0.7) * 1.2;
  } else if (kind == CAFE) {
    c = vec3f(1.0, 0.65, 0.35) * 0.55;
  } else if (kind == BAR) {
    c = neon(h21(id + 5.5)) * 0.25;
  } else if (kind == VENDING) {
    c = mix(vec3f(0.9, 0.95, 1.0), vec3f(1.0, 0.25, 0.2), step(0.5, h21(id + 2.2))) * 0.5;
  }
  return c;
}

// Lantern strings hang across the street over every other row (not over cross streets).
fn string_z(m: f32) -> f32 {
  return m * 2.0 * UNIT + 2.1;
}

fn string_on(m: f32) -> bool {
  let n = floor(string_z(m) / UNIT);
  return !crossing(n) && lantern_block(n) && h21(vec2f(m, 31.7)) < 0.85;
}

fn string_y(m: f32, x: f32) -> f32 {
  let x1 = x / HALF;
  return 5.2 + 0.9 * h21(vec2f(m, 2.3)) - 0.55 * (1.0 - x1 * x1);
}

fn lantern_colour(m: f32) -> vec3f {
  return select(vec3f(1.0, 0.3, 0.15) * 2.4, vec3f(1.0, 0.82, 0.58) * 2.2, h21(vec2f(m, 5.9)) < 0.35);
}

// ---- The grid ----

struct Hit {
  t: f32,
  // 0 nothing, 1 a face across, 2 a face along, 3 a roof.
  kind: f32,
  i: i32,
  n: f32,
  // 0 the building, 1 the billboard on its roof, 2 a torii on its roof (and which piece: 2 a
  // pillar, 3 the nuki, 4 the kasagi).
  part: f32,
}

fn trace(o: vec3f, d: vec3f, steps: f32, tmax: f32) -> Hit {
  var hit: Hit;
  hit.t = tmax;
  hit.kind = 0.0;
  hit.part = 0.0;
  let inv = inverse(d);
  var i = column(o.x);
  var n = floor(o.z / UNIT);
  var t = 0.0;
  // Along the street (column 0) nothing stands: the rows the ray surely runs along it before it
  // leaves it (or reaches tmax) are passed over at once, as the steps below would, one by one.
  var first = 0.0;
  if (i == 0 && d.z > 1e-3) {
    var tx = 1e9;
    if (d.x > 1e-6) {
      tx = (HALF - o.x) / d.x;
    } else if (d.x < -1e-6) {
      tx = (-HALF - o.x) / d.x;
    }
    // A row short of the one it leaves in, so that rounding never takes it too far.
    first = max(floor((o.z + d.z * min(tx, tmax)) / UNIT) - 1.0 - n, 0.0);
    n += first;
  }
  for (var k = first; k < 72.0; k += 1.0) {
    if (k >= steps) {
      break;
    }
    let span = column_span(i);
    var tx = 1e9;
    if (d.x > 1e-6) {
      tx = (span.y - o.x) / d.x;
    } else if (d.x < -1e-6) {
      tx = (span.x - o.x) / d.x;
    }
    var tz = 1e9;
    if (d.z > 1e-6) {
      tz = ((n + 1.0) * UNIT - o.z) / d.z;
    } else if (d.z < -1e-6) {
      tz = (n * UNIT - o.z) / d.z;
    }
    let exit = min(tx, tz);
    let h = height_of(i, n);
    if (h > 0.0) {
      let b = cell_box(i, n);
      let r = box_hit(o, inv, vec3f(b.x, 0.0, b.z), vec3f(b.y, h, b.w));
      if (r.x > 0.0 && r.x < hit.t) {
        hit.t = r.x;
        hit.kind = r.y;
        hit.i = i;
        hit.n = n;
      }
      if (abs(i) == 1) {
        let s = f32(i);
        let bb = billboard_of(n, s);
        if (bb.y > 0.0) {
          let w = wall_x(n, s);
          let z0 = first_row(n, s) * UNIT + 0.4;
          let z1 = z0 + rows_of(n, s) * UNIT - 0.8;
          let x0 = select(-w - 0.3, w, s > 0.0);
          let rb = box_hit(o, inv, vec3f(x0, bb.x, z0), vec3f(x0 + 0.3, bb.y, z1));
          if (rb.x > 0.0 && rb.x < hit.t) {
            hit.t = rb.x;
            hit.kind = rb.y;
            hit.i = i;
            hit.n = n;
            hit.part = 1.0;
          }
        }
      }
      let shrine = torii_of(i, n);
      if (shrine.z > 0.0) {
        let rt = torii_hit(o, inv, shrine);
        if (rt.x > 0.0 && rt.x < hit.t) {
          hit.t = rt.x;
          hit.kind = rt.y;
          hit.i = i;
          hit.n = n;
          hit.part = 2.0 + rt.z;
        }
      }
      if (hit.kind > 0.5) {
        return hit;
      }
    }
    if (exit >= tmax) {
      break;
    }
    t = exit;
    if (tx < tz) {
      i += select(-1, 1, d.x > 0.0);
    } else {
      n += select(-1.0, 1.0, d.z > 0.0);
    }
  }
  return hit;
}

// ---- Words on signs ----
// The words are lettered in eight faces (glyphs.wgsl names them): a plain heavy gothic for the
// street's own lettering (the arch, the banners, the fascias of the chains), and the art faces
// the shops choose from, each sign in its own as on a real street: a heavy display gothic, a
// rough brush-like one, an outlined one with its shadow, a round one, an antique mincho, a marker
// hand and dots. As Persona 5 letters its street, the words are few and big: a word fills its
// sign, or one or two characters do.

// How much of a pixel character ci of word w covers at g (0..1 across its cell), in `face`.
// `vertical` takes the upright forms; aa is a pixel in cell units. With outline > 0 only a tube
// that wide (in cell units) along the outline, as neon letters are.
fn glyph_ink(face: f32, w: f32, ci: f32, g: vec2f, vertical: bool, aa: f32, outline: f32) -> f32 {
  let size = vec2f(textureDimensions(asset_glyphs));
  var id = (word_code(w) >> (8u * u32(clamp(ci, 0.0, 3.0)))) & 0xffu;
  if (vertical && id == GLYPH_LONG) {
    id = GLYPH_LONG_V;
  }
  // Faces 0 to 3 are the channels of the atlas's top block, 4 to 7 of the one under it.
  let f = u32(clamp(face, 0.0, 7.0));
  let cell = vec2f(f32(id % GLYPH_COLUMNS), f32(id / GLYPH_COLUMNS) + f32(f / 4u) * GLYPH_BLOCK);
  let q = clamp(g, vec2f(0.02), vec2f(0.98));
  let uv = (cell + q) * GLYPH_CELL / size;
  let lod = max(log2(aa * GLYPH_CELL), 0.0);
  let channel = select(vec4f(0.0), vec4f(1.0), vec4u(f % 4u) == vec4u(0u, 1u, 2u, 3u));
  let d = dot(textureSampleLevel(asset_glyphs, linear_clamp, uv, lod), channel);
  // The field changes by 1 / (2 * spread) per atlas pixel.
  let per = 1.0 / (2.0 * GLYPH_SPREAD);
  let soft = max(aa * GLYPH_CELL * per * 0.7, 0.008);
  let inside = step(0.0, g.x) * step(g.x, 1.0) * step(0.0, g.y) * step(g.y, 1.0);
  if (outline > 0.0) {
    let half = outline * GLYPH_CELL * per;
    return smoothstep(half + soft, max(half - soft, 0.0), abs(d - 0.5)) * inside;
  }
  return smoothstep(0.5 - soft, 0.5 + soft, d) * inside;
}

// The face shop `id` letters its signs in: mostly the art faces, the heavy gothic most of all.
fn face_of(id: vec2f) -> f32 {
  let r = h21(id + 17.3);
  var f = FACE_DOT;
  f = select(f, FACE_MAGIC, r < 0.94);
  f = select(f, FACE_ANTIQUE, r < 0.86);
  f = select(f, FACE_ROCKNROLL, r < 0.7);
  f = select(f, FACE_RAMPART, r < 0.56);
  f = select(f, FACE_REGGAE, r < 0.46);
  f = select(f, FACE_DELA, r < 0.3);
  return select(f, FACE_NOTO, r < 0.06);
}

// The word sign `id` shows, one of the shops' words (glyphs.wgsl) that fits in `room` characters.
fn word_for(id: vec2f, room: f32) -> f32 {
  for (var k = 0.0; k < 4.0; k += 1.0) {
    let i = floor(h21(id + vec2f(k * 5.1, 2.2)) * (SIGN_WORDS + MORE_WORDS));
    let w = select(i, MORE_FIRST + i - SIGN_WORDS, i >= SIGN_WORDS);
    if (word_length(w) <= room) {
      return w;
    }
  }
  // The one-character words: the characters for medicine, book and pawn.
  let r = h21(id + 9.4);
  return select(select(5.0, 23.0, r < 0.6), 27.0, r < 0.25);
}

// The big characters on sign `id`: one, or with `pair` two.
fn big_word(id: vec2f, pair: bool) -> f32 {
  let r = h21(id + 25.3);
  return BIG_FIRST + select(floor(r * BIG_SINGLE), BIG_SINGLE + floor(r * (BIG_WORDS - BIG_SINGLE)), pair);
}

// Coverage of word w written down a strip `width` wide; x, v in metres from its top left, the
// characters centred in `height`.
fn vertical_word(face: f32, w: f32, x: f32, v: f32, width: f32, height: f32, aa: f32, outline: f32) -> f32 {
  let len = word_length(w);
  let top = 0.5 * (height - len * width);
  let ci = floor((v - top) / width);
  let g = vec2f(x / width, (v - top) / width - ci);
  return glyph_ink(face, w, ci, g, true, aa / width, outline) * step(0.0, ci) * step(ci, len - 1.0);
}

// Coverage of word w written along a band `height` tall, centred in `width`.
fn horizontal_word(face: f32, w: f32, x: f32, y: f32, width: f32, height: f32, aa: f32, outline: f32) -> f32 {
  let len = word_length(w);
  let left = 0.5 * (width - len * height);
  let ci = floor((x - left) / height);
  let g = vec2f((x - left) / height - ci, y / height);
  return glyph_ink(face, w, ci, g, false, aa / height, outline) * step(0.0, ci) * step(ci, len - 1.0);
}

// A lettered sign, the one design every shop's sign keeps to: its word in big letters, as big as
// the sign allows, down it (`vertical`) or along it, in the shop's face; x, y in metres from its
// top left; its colour lit at 1 (the caller sets how bright). Styles: 0 a lightbox, brighter in
// the middle, the letters dark on a light colour or white on a deep one; 1 a dark board, the
// letters lit in the colour; 2 a white box, the letters in the colour; 3 the colour, the letters
// white with a dark shadow cast down and to the right. `frame`: 0 none (a shape cut by the
// caller), 1 a dark rim and inside it a second line (lit in style 1, the colour in 2, white in 0
// and 3), 2 the rim and a frame of lit bulbs inside it. `inset` keeps the letters further in.
fn lettered(id: vec2f, w: f32, face: f32, vertical: bool, x: f32, y: f32, width: f32, height: f32, style: f32, frame: f32, inset: f32, aa: f32) -> vec3f {
  let bg = neon(h21(id + 4.7));
  let small = min(width, height);
  let rim = select(0.0, 0.05 * small, frame > 0.5);
  let bulbs = select(0.0, 0.11, frame > 1.5);
  let m = rim + bulbs + 0.06 * small + inset;
  let len = word_length(w);
  var ink: f32;
  var shadow = 0.0;
  // Only style 3 casts a shadow: the second look into the atlas is taken only there.
  let off = select(0.0, 0.07, style > 2.5);
  if (vertical) {
    let cell = min(width - 2.0 * m, (height - 2.0 * m) / len);
    let x0 = 0.5 * (width - cell);
    ink = vertical_word(face, w, x - x0, y - m, cell, height - 2.0 * m, aa, 0.0);
    if (off > 0.0) {
      shadow = vertical_word(face, w, x - x0 - off * cell, y - m - off * cell, cell, height - 2.0 * m, aa, 0.0);
    }
  } else {
    let cell = min(height - 2.0 * m, (width - 2.0 * m) / len);
    let y0 = 0.5 * (height - cell);
    ink = horizontal_word(face, w, x - m, y - y0, width - 2.0 * m, cell, aa, 0.0);
    if (off > 0.0) {
      shadow = horizontal_word(face, w, x - m - off * cell, y - y0 - off * cell, width - 2.0 * m, cell, aa, 0.0);
    }
  }
  let edge = min(min(x, width - x), min(y, height - y));
  let line = select(0.0, smoothstep(0.018 + aa, 0.0, abs(edge - rim - bulbs - 0.035)), frame > 0.5);
  let white = vec3f(1.6, 1.55, 1.45);
  var col: vec3f;
  if (style < 1.0) {
    let across = select(y / height, x / width, vertical);
    let fill = bg * 1.2 * (0.75 + 0.35 * (1.0 - pow(2.0 * across - 1.0, 2.0)));
    col = mix(mix(fill, select(white, vec3f(0.02), rv_luma(bg) > 0.4), ink), white, line);
  } else if (style < 2.0) {
    col = vec3f(0.012) + bg * 2.6 * max(ink, line);
  } else if (style < 3.0) {
    col = mix(vec3f(1.25, 1.2, 1.1), bg * 1.3, max(ink, line));
  } else {
    col = mix(mix(bg * 1.1, vec3f(0.02), shadow), white, max(ink, line));
  }
  if (bulbs > 0.0 && edge < rim + bulbs) {
    // Lit and steady, every 11 cm round the sign.
    let along = select(x, y, min(x, width - x) > min(y, height - y));
    let f = vec2f((fract(along / 0.11) - 0.5) * 0.11, edge - rim - 0.055);
    col = vec3f(0.03) + vec3f(2.6, 2.1, 1.3) * smoothstep(0.032 + aa, 0.016, length(f));
  }
  return mix(col, vec3f(0.035), smoothstep(rim * 0.7 + aa, rim * 0.7, edge) * step(0.5, frame));
}

// A few tubes are failing: they flicker.
fn flicker(id: vec2f) -> f32 {
  let failing = step(0.94, h21(id + 7.3));
  return mix(1.0, 0.15 + 0.85 * step(0.35, rv_value_noise(vec2f(frame.time * 9.0, id.x * 7.0 + id.y))), failing);
}

fn sign_face(n: f32, s: f32, p: vec3f, aa: f32) -> vec3f {
  let id = vec2f(n, s);
  let sg = sign_of(n, s);
  let w = sg.y;
  let h = sg.w - sg.z;
  // Across as read from the street, and down from the top.
  let a = (wall_x(n, s) - 0.12) - s * p.x;
  let x = select(a, w - a, s > 0.0);
  let v = sg.w - p.y;
  let bg = neon(h21(id + 4.7));
  // Older signs are dimmer.
  let glow = glow_now() * (0.7 + 0.45 * h21(id + 2.8)) * flicker(id);
  let face = face_of(id);
  let style = h21(id + 6.1);
  var col: vec3f;
  if (style < 0.5) {
    // A lettered box (lettered()), its word down it or one or two big characters filling it,
    // now and then in a frame of bulbs.
    let word = select(word_for(id, 4.0), big_word(id, h21(id + 9.7) < 0.5), h21(id + 9.9) < 0.4);
    let frame = select(1.0, 2.0, h21(id + 13.1) < 0.2);
    col = lettered(id, word, face, true, x, v, w, h, floor(h21(id + 3.6) * 4.0), frame, 0.0, aa) * glow;
  } else if (style < 0.66) {
    // Neon letters on a dark board, a tube round its edge.
    let m = 0.08 * w;
    let cell = min(w - 2.0 * m, (h - 2.0 * m) / 2.0);
    let word = word_for(id, floor((h - 2.0 * m) / cell));
    let tube = vertical_word(face, word, x - m, v - m, cell, h - 2.0 * m, aa, 0.045);
    let edge = min(min(x, w - x), min(v, h - v));
    let border = smoothstep(0.025 + aa, 0.0, abs(edge - m * 0.45));
    col = vec3f(0.012, 0.011, 0.012) + bg * glow * 3.0 * max(tube, border);
  } else if (style < 0.9) {
    // A stack of boxes, one per tenant floor, each its own colour, face and word.
    let count = max(floor(h / 1.3), 1.0);
    let seg = h / count;
    let si = min(floor(v / seg), count - 1.0);
    let sid = id + vec2f(si * 3.1, 7.7);
    let sv = v - si * seg;
    let word = select(word_for(sid, 4.0), big_word(sid, false), h21(sid + 9.9) < 0.3);
    col = lettered(sid, word, face_of(sid), true, x, sv, w, seg, floor(h21(sid + 3.6) * 4.0), 1.0, 0.0, aa) * glow * (0.85 + 0.3 * h21(sid + 1.0));
  } else {
    // An amber dot-matrix board scrolling on at the song's pace (`scroll`, a step a beat).
    let pitch = 0.075;
    let dot_id = floor(vec2f(x, v) / pitch);
    let f = fract(vec2f(x, v) / pitch) - 0.5;
    let pattern = pattern_of(id);
    let on = step(0.5, fract((dot_id.y + dot_id.x * 0.7) / 6.0 - state.scroll * 0.5 + pattern * 0.13));
    let led = mix(smoothstep(0.42, 0.25, length(f)), 0.45, smoothstep(0.3, 1.0, aa / pitch));
    col = vec3f(0.01) + vec3f(1.0, 0.45, 0.06) * 2.6 * glow * on * led;
  }
  return mix(col * 0.03 + vec3f(0.01), col, lit(id));
}

// A board along a facade with word w on it in `face`; x, y in metres from its top left. Styles:
// 0 a lit board with dark letters, 1 lit letters on a dark board, 2 white letters on dark wood.
fn board(id: vec2f, w: f32, face: f32, style: f32, x: f32, y: f32, width: f32, height: f32, light: vec3f, aa: f32) -> vec3f {
  let bg = neon(h21(id + 4.7));
  let glow = glow_now() * (0.75 + 0.4 * h21(id + 2.8)) * flicker(id);
  let ink = horizontal_word(face, w, x, y - 0.1 * height, width, height * 0.8, aa, 0.0);
  var col: vec3f;
  if (style < 0.5) {
    let text = select(vec3f(1.5) * glow, vec3f(0.02), rv_luma(bg) > 0.4);
    col = mix(bg * 0.95 * glow, text, ink);
  } else if (style < 1.5) {
    col = vec3f(0.015) + bg * 2.6 * glow * ink;
  } else {
    let wood = vec3f(0.22, 0.11, 0.05) * (light * 3.0 + vec3f(0.01));
    col = mix(wood, vec3f(1.2, 1.1, 0.95) * (0.4 + light * 6.0), ink);
  }
  let edge = min(min(x, width - x), min(y, height - y));
  return mix(col, vec3f(0.04), smoothstep(0.03 + aa, 0.03, edge));
}

// A panel along a facade; x, y in metres from its top left: a screen showing the spectrum, or a
// lettered board (lettered()), its word along it or one or two big characters.
fn panel(id: vec2f, x: f32, y: f32, w: f32, h: f32, aa: f32) -> vec3f {
  let glow = glow_now();
  var col: vec3f;
  if (h > 1.0 && h21(id + 2.9) < 0.25) {
    // A screen: bars of the spectrum, in LED cells.
    let nc = neon(h21(id + 4.7));
    let nc2 = neon(h21(id + 8.8));
    let bars = 32.0;
    let b = floor(x / w * bars);
    let level = rv_spectrum(0.06 + 0.8 * (b + 0.5) / bars, 0.0);
    let height = 1.0 - y / h;
    let cell = rv_band(fract(x / w * bars), 0.12, 0.88, aa * bars / w) * rv_band(fract(y / h * 16.0), 0.15, 0.85, aa * 16.0 / h);
    let on = step(height, level * level * 1.2) * cell;
    col = vec3f(0.006, 0.006, 0.01) + mix(nc, nc2, height) * 2.4 * on * glow;
  } else {
    let word = select(word_for(id, floor(w / (h * 0.6))), big_word(id, h21(id + 9.7) < 0.6), h21(id + 9.9) < 0.5);
    let frame = select(1.0, 2.0, h21(id + 13.1) < 0.25);
    col = lettered(id, word, face_of(id), false, x, y, w, h, floor(h21(id + 3.6) * 4.0), frame, 0.0, aa) * glow * (0.75 + 0.4 * h21(id + 2.8)) * flicker(id);
  }
  return mix(col * 0.03 + vec3f(0.01), col, lit(id));
}

// A big screen; x, y in metres from its top left. What it shows changes every 16 hits that are
// heard: the spectrum as bars, a ring from the middle on every hit, or bands sweeping on at the
// song's pace. In a fade the ring weakens (and the hits are counted less), in silence nothing
// moves or changes.
fn big_screen(id: vec2f, x: f32, y: f32, w: f32, h: f32, aa: f32) -> vec3f {
  let mode = floor(h21(id + vec2f(floor(state.changes / 16.0), 4.2)) * 3.0);
  let uv = vec2f(x / w, y / h);
  let nc = neon(h21(id + 4.7));
  let nc2 = neon(h21(id + 8.8));
  var c: vec3f;
  if (mode < 1.0) {
    let bars = 24.0;
    let b = floor(uv.x * bars);
    let level = rv_spectrum(0.05 + 0.8 * (b + 0.5) / bars, 0.0);
    let up = 1.0 - uv.y;
    c = mix(nc, nc2, up) * step(up, level * level * 1.3) * rv_band(fract(uv.x * bars), 0.1, 0.9, aa * bars / w);
  } else if (mode < 2.0) {
    let r = length((uv - 0.5) * vec2f(w / h, 1.0));
    let since = frame.time - state.ring_time;
    let ring = (exp(-abs(r - since * 1.2) * 18.0) + 0.5 * exp(-abs(r - (since + state.ring_gap) * 1.2) * 18.0)) * input.audible;
    c = mix(nc, nc2, smoothstep(0.0, 0.8, r)) * ring + nc2 * 0.05;
  } else {
    let band = fract(uv.x * 3.0 + uv.y * 1.5 - state.scroll * 0.25);
    c = mix(nc, nc2, step(0.5, band)) * (0.3 + 0.7 * smoothstep(0.5, 0.0, abs(band - 0.5)));
  }
  let cell = 0.1;
  let px = rv_band(fract(x / cell), 0.1, 0.9, aa / cell) * rv_band(fract(y / cell), 0.1, 0.9, aa / cell);
  let pixels = mix(px, 0.8, smoothstep(0.3, 1.0, aa / cell));
  return vec3f(0.004) + c * 1.8 * glow_now() * pixels * lit(id);
}

// ---- Buildings ----

fn albedo_of(id: vec2f) -> vec3f {
  let k = h21(id + 3.1);
  if (k < 0.3) {
    return vec3f(0.3, 0.29, 0.28);
  }
  if (k < 0.55) {
    return vec3f(0.42, 0.36, 0.3);
  }
  if (k < 0.75) {
    return vec3f(0.08, 0.09, 0.11);
  }
  return vec3f(0.4, 0.17, 0.13);
}

// Floors of windows on any wall; u metres along it, y up, `light` what falls on it.
// k picks the style: below 0.3 plain, to 0.55 tiled, to 0.75 a glass curtain wall, above 0.88
// with balconies.
// The wall is not flat (`relief`): at the foot of each floor a band stands out, its top catching
// the sky's cool light and the wall under it in its shade; the windows are set in, the top of
// each opening in shadow and its sides darker, a sill standing out under it, lit on top with a
// shadow beneath; the parapet's coping is lit along its top.
fn storeys(id: vec2f, k: f32, u: f32, y: f32, hb: f32, light: vec3f, aa: f32) -> vec3f {
  let albedo = albedo_of(id);
  var c = albedo * light;
  let fl = floor(y / STOREY);
  let fy = y / STOREY - fl;
  let relief = params.relief;
  let sky = params.ambient * vec3f(0.75, 0.85, 1.25) * 3.0;
  let ay = aa / STOREY;
  if (y > hb - 0.6) {
    // The parapet.
    return c * 0.7 + vec3f(0.01) * rv_band(y, hb - 0.12, hb, aa) + sky * albedo * relief * rv_band(y, hb - 0.05, hb, aa);
  }
  if (k >= 0.55 && k < 0.75) {
    // A glass curtain wall: dark glass with mullions, whole floors lit.
    let mull = rv_band(fract(u / 1.2), 0.04, 0.96, aa / 1.2) * rv_band(fy, 0.08, 0.95, aa / STOREY);
    let office = step(h21(vec2f(fl, id.x * 3.0 + id.y)), 0.3);
    let glass = mix(vec3f(0.01, 0.014, 0.025), vec3f(0.55, 0.65, 0.8) * 0.5, office);
    return mix(c * 0.6, glass, mull);
  }
  // Rain has streaked the walls under the windows.
  c *= 1.0 - 0.2 * smoothstep(0.55, 0.85, rv_value_noise(vec2f(u * 2.5 + id.x * 13.0, y * 0.12 + id.y)));
  // The floor's band, standing out at its foot: lighter, its top lit, the wall under the next
  // floor's band in shade.
  c *= 1.0 + 0.25 * relief * rv_band(fy, 0.0, 0.09, ay);
  c += sky * albedo * relief * rv_band(fy, 0.075, 0.09, ay);
  c *= 1.0 - 0.45 * relief * rv_band(fy, 0.94, 1.0, ay);
  // Windows, some lit (behind curtains); tiled walls show their joints; some buildings have
  // balconies.
  if (k < 0.55 && k > 0.3) {
    c *= 0.85 + 0.15 * rv_band(fract(u / 0.3), 0.06, 0.94, aa / 0.3) * rv_band(fract(y / 0.15), 0.08, 0.92, aa / 0.15);
  }
  let wi = floor(u / 1.5);
  let win = rv_band(fract(u / 1.5), 0.15, 0.85, aa / 1.5) * rv_band(fy, 0.25, 0.8, aa / STOREY);
  let on = h21(vec2f(id.x * 5.0 + wi + id.y * 7.0, fl * 3.0));
  let curtain = 0.55 + 0.45 * rv_value_noise(vec2f(u * 3.0, fl * 7.0 + id.x));
  let lamp = select(vec3f(1.0, 0.72, 0.45), vec3f(0.75, 0.85, 1.0), on > 0.2) * 0.45 * curtain;
  let glass = select(vec3f(0.006, 0.007, 0.01) + light * 0.1, lamp, on < 0.3);
  // The sill under the window, standing out: lighter, its top lit, a shadow under it.
  let fu = fract(u / 1.5);
  let sill_span = rv_band(fu, 0.12, 0.88, aa / 1.5);
  c *= 1.0 + 0.2 * relief * sill_span * rv_band(fy, 0.2, 0.25, ay);
  c += sky * albedo * relief * sill_span * rv_band(fy, 0.24, 0.25, ay);
  c *= 1.0 - 0.4 * relief * sill_span * rv_band(fy, 0.15, 0.2, ay);
  // The opening set in: its top in shadow, its sides darker.
  let reveal = max(rv_band(fy, 0.72, 0.8, ay) * 0.6, max(rv_band(fu, 0.15, 0.19, aa / 1.5), rv_band(fu, 0.81, 0.85, aa / 1.5)) * 0.35);
  c = mix(c, glass * (1.0 - relief * reveal), win);
  if (k > 0.88) {
    let rail = rv_band(fy, 0.0, 0.22, aa / STOREY) * (0.6 + 0.4 * rv_band(fract(u / 0.12), 0.0, 0.25, aa / 0.12));
    c = mix(c, vec3f(0.5) * light + vec3f(0.004), rail * 0.8);
  }
  return c;
}

// The ground floor of shop (n, s) up to its fascia: u metres along it, `along` as read from the
// street, y up, `light` what falls on it.
fn shop_front(n: f32, s: f32, u: f32, along: f32, y: f32, light: vec3f, aa: f32) -> vec3f {
  let id = vec2f(n, s);
  let kind = shop_kind(n, s);
  let shop = shop_light(n, s);
  let wall = albedo_of(id) * light;
  let glow = glow_now();
  if (y >= 2.45) {
    // The fascia.
    let t = (y - 2.45) / 0.65;
    if (kind == KONBINI) {
      // Three stripes in the chain's colours.
      let chain = floor(h21(id + 1.9) * 3.0);
      let a = select(select(vec3f(0.1, 0.35, 1.0), vec3f(1.0, 0.45, 0.05), chain == 1.0), vec3f(0.1, 0.75, 0.3), chain == 2.0);
      let b = select(vec3f(1.0, 0.2, 0.15), vec3f(0.15, 0.4, 1.0), chain == 2.0);
      return mix(mix(a, vec3f(1.0), step(0.33, t)), b, step(0.66, t)) * 0.9 * glow;
    }
    let fx = along - 0.15;
    let fy = 3.05 - y;
    if (kind == GAMES) {
      // The katakana word for "game" in white between rows of chasing bulbs.
      let k = floor(along / 0.15);
      let f = vec2f(fract(along / 0.15) - 0.5, (fract(t * 2.0) - 0.5) * 2.0);
      let on = step(0.6, fract((k + floor(frame.time * 10.0)) / 3.0));
      let bulb = smoothstep(0.35, 0.15, length(f)) * (1.0 - rv_band(t, 0.2, 0.8, 0.0));
      let ink = horizontal_word(FACE_DELA, 10.0, fx, fy - 0.12, UNIT - 0.3, 0.32, aa, 0.0);
      return neon(h21(id + 4.7)) * 0.2 * glow + vec3f(3.0, 2.6, 1.8) * bulb * (0.15 + 0.85 * on) * glow + vec3f(1.6) * ink * glow;
    }
    if (kind == DRUGSTORE) {
      // The one-character sign for medicine, big and red on yellow.
      let ink = horizontal_word(FACE_DELA, 5.0, fx, fy - 0.03, UNIT - 0.3, 0.5, aa, 0.0);
      return mix(vec3f(1.0, 0.8, 0.05), vec3f(0.9, 0.05, 0.04), ink) * glow;
    }
    if (kind == IZAKAYA) {
      // Its word in white on dark wood, under a small lamp.
      var words = array<f32, 4>(2.0, 8.0, 7.0, 3.0);
      let w = words[u32(h21(id + 3.3) * 3.99)];
      return board(id, w, FACE_ANTIQUE, 2.0, fx, fy, UNIT - 0.3, 0.55, light + shop * 0.08 * exp(-abs(along - 0.5 * UNIT)), aa);
    }
    if (kind == CAFE) {
      let w = select(6.0, 20.0, h21(id + 3.3) < 0.5);
      return board(id, w, FACE_MAGIC, 2.0, fx, fy, UNIT - 0.3, 0.55, light + shop * 0.05, aa);
    }
    if (kind == BAR) {
      return wall * 0.5 + board(id, 24.0, face_of(id), 1.0, fx, fy, UNIT - 0.3, 0.55, light, aa) * lit(id);
    }
    if (kind == BOUTIQUE) {
      return wall * 0.5 + neon(h21(id + 4.7)) * 2.5 * rv_band(y, 2.74, 2.78, aa) * glow * lit(id);
    }
    if (kind == SHUTTER || kind == VENDING) {
      return wall;
    }
    return panel(id + 0.37, along - 0.15, 3.05 - y, UNIT - 0.3, 0.55, aa);
  }

  if (kind == KONBINI) {
    // Glass from end to end, bright inside: shelves, magazines along the window, ceiling lights.
    let mull = 1.0 - rv_band(fract(u / 1.5), 0.03, 0.97, aa / 1.5) * rv_band(y, 0.15, 2.4, aa);
    var c = shop * 0.55;
    let goods = rv_band(fract(y / 0.42), 0.12, 0.8, aa / 0.42) * rv_band(fract(u / 0.16), 0.08, 0.92, aa / 0.16)
      * step(h21(vec2f(floor(u / 0.16) + n * 31.0, floor(y / 0.42) + s * 7.0)), 0.8) * rv_band(y, 0.9, 1.95, aa);
    c = mix(c, hsv(h21(vec2f(floor(u / 0.16) * 0.37 + n, floor(y / 0.42))), 0.35, 1.0) * 0.8, goods * smoothstep(0.08, 0.02, aa));
    let magazines = rv_band(y, 0.35, 0.85, aa) * rv_band(fract(u / 0.22), 0.1, 0.9, aa / 0.22);
    c = mix(c, hsv(h21(vec2f(floor(u / 0.22), n)), 0.6, 1.0) * 0.8, magazines * 0.8);
    c += vec3f(2.0) * rv_band(y, 2.25, 2.32, aa);
    return mix(c, vec3f(0.05), mull * 0.9);
  }
  if (kind == IZAKAYA) {
    // Dark wood; a lattice over a warm window; a door hung with a noren; a red lantern.
    var c = vec3f(0.12, 0.06, 0.03) * (light * 2.0 + vec3f(0.02));
    let window = rv_band(u, 0.25, 1.2, aa) * rv_band(y, 0.5, 2.1, aa);
    let slat = rv_band(fract(u / 0.09), 0.25, 0.75, aa / 0.09);
    c = mix(c, mix(shop * 0.9, vec3f(0.08, 0.04, 0.02), slat), window);
    let doorway = rv_band(u, 1.8, 3.4, aa) * rv_band(y, 0.0, 2.3, aa);
    c = mix(c, shop * 0.6, doorway);
    let pf = fract((u - 1.8) / 0.533);
    let noren = rv_band(u, 1.8, 3.4, aa) * rv_band(y, 1.55, 2.3, aa) * rv_band(pf, 0.04, 0.96, aa / 0.533);
    let cloth = select(vec3f(0.05, 0.08, 0.3), vec3f(0.5, 0.05, 0.04), h21(id + 3.7) < 0.5);
    let emblem = smoothstep(0.17 + aa, 0.17 - aa, length(vec2f(u - 2.6, y - 1.95)));
    c = mix(c, mix(cloth, vec3f(0.85), emblem) * (light * 3.0 + shop * 0.5), noren);
    let lp = vec2f((u - 1.45) / 0.2, (y - 1.75) / 0.3);
    let lantern = smoothstep(1.0 + aa * 5.0, 1.0, length(lp));
    return mix(c, vec3f(2.4, 0.25, 0.08) * (1.1 - 0.5 * dot(lp, lp)), lantern);
  }
  if (kind == GAMES) {
    // Wide open, rows of machines flickering inside, a frame of chasing bulbs.
    let open = rv_band(u, 0.3, UNIT - 0.3, aa) * rv_band(y, 0.0, 2.3, aa);
    let ci = floor(u / 0.45);
    let flicker = 0.6 + 0.4 * sin(frame.time * (6.0 + 7.0 * h21(vec2f(ci, n))) + ci);
    let machine = rv_band(fract(u / 0.45), 0.12, 0.88, aa / 0.45) * rv_band(y, 0.4, 1.7, aa);
    var c = mix(wall, shop * 0.3 + neon(h21(vec2f(ci, n) + 2.0)) * 1.4 * machine * flicker, open);
    let top = vec2f(fract(u / 0.15) - 0.5, (y - 2.38) / 0.15);
    let side = vec2f(min(abs(u - 0.18), abs(u - (UNIT - 0.18))) / 0.15, fract(y / 0.15) - 0.5);
    let bulb = max(smoothstep(0.3, 0.1, length(top)) * rv_band(u, 0.15, UNIT - 0.15, 0.0), smoothstep(0.3, 0.1, length(side)) * step(y, 2.38));
    let on = step(0.6, fract((floor(u / 0.15) + floor(y / 0.15) + floor(frame.time * 10.0)) / 3.0));
    return c + vec3f(3.0, 2.5, 1.6) * bulb * (0.15 + 0.85 * on) * glow;
  }
  if (kind == BOUTIQUE) {
    // A dark room with figures standing in spotlights.
    var c = shop * 0.05;
    for (var k = 0.0; k < 3.0; k += 1.0) {
      let x = 0.9 + k * 1.35;
      let cone = exp(-pow((u - x) / (0.25 + 0.3 * (2.4 - y) / 2.4), 2.0));
      c += shop * 0.8 * cone * smoothstep(0.0, 0.5, y);
      let body = rv_sd_round_box(vec2f(u - x, y - 1.0), vec2f(0.17, 0.75), 0.15);
      let head = length(vec2f(u - x, y - 1.95)) - 0.12;
      c = mix(c, vec3f(0.9, 0.86, 0.8) * shop * 2.5, smoothstep(aa, -aa, min(body, head)));
    }
    return mix(wall, c, rv_band(u, 0.2, UNIT - 0.2, aa) * rv_band(y, 0.1, 2.4, aa));
  }
  if (kind == DRUGSTORE) {
    // Bright inside, the window covered in price boards.
    var c = shop * 0.6;
    let cell = floor(vec2f(u / 0.55, y / 0.45));
    let f = fract(vec2f(u / 0.55, y / 0.45));
    let board = rv_band(f.x, 0.06, 0.94, aa / 0.55) * rv_band(f.y, 0.08, 0.92, aa / 0.45) * step(h21(cell + id), 0.6) * rv_band(y, 0.45, 2.3, aa);
    let colour = select(vec3f(1.0, 0.85, 0.05), vec3f(1.0, 0.12, 0.1), h21(cell + id + 0.5) < 0.4);
    let price = smoothstep(aa, -aa, rv_sd_box(f - 0.5, vec2f(0.25, 0.08)));
    return mix(c, mix(colour, vec3f(0.05), price * 0.8), board);
  }
  if (kind == CAFE) {
    // A wooden frame round a warm window: pendant lamps, tables.
    var c = vec3f(0.18, 0.1, 0.05) * (light * 2.0 + vec3f(0.01));
    var inside = shop * 0.35 * (0.5 + 0.5 * smoothstep(0.3, 2.3, y));
    let lamp_at = vec2f(0.6 + round((u - 0.6) / 0.9) * 0.9, 1.95);
    let r = length(vec2f(u, y) - lamp_at);
    inside += vec3f(3.0, 2.0, 1.1) * smoothstep(0.09 + aa, 0.09 - aa, r) + vec3f(1.0, 0.6, 0.3) * 0.25 * exp(-r * 3.0);
    inside = mix(inside, vec3f(0.02), smoothstep(0.008 + aa, 0.0, abs(u - lamp_at.x)) * step(2.0, y));
    let table = rv_band(y, 0.72, 0.78, aa) * rv_band(fract(u / 1.4), 0.2, 0.75, aa / 1.4);
    inside = mix(inside, vec3f(0.03, 0.02, 0.015), table);
    return mix(c, inside, rv_band(u, 0.2, UNIT - 0.2, aa) * rv_band(y, 0.3, 2.35, aa));
  }
  if (kind == BAR) {
    // Almost black: a small glowing window, a door framed in neon.
    var c = vec3f(0.02, 0.018, 0.022) + wall * 0.3;
    let nc = neon(h21(id + 4.7));
    c = mix(c, shop * 0.8 + nc * 0.1, rv_band(u, 0.5, 1.3, aa) * rv_band(y, 1.2, 1.9, aa));
    let d = rv_sd_box(vec2f(u - 2.6, y - 1.1), vec2f(0.5, 1.1));
    c = mix(c, shop * 0.15, smoothstep(aa, -aa, d));
    return c + nc * 3.0 * smoothstep(0.035 + aa, 0.0, abs(d - 0.06)) * glow * lit(id);
  }
  if (kind == VENDING) {
    let mu = fract(u / 0.95);
    let inside = rv_band(mu, 0.05, 0.95, aa / 0.95) * rv_band(y, 0.0, 1.85, aa);
    let rows = rv_band(fract(y * 3.0), 0.2, 0.8, aa * 3.0) * rv_band(y, 0.9, 1.8, aa) * rv_band(mu, 0.15, 0.85, aa / 0.95);
    return mix(wall * 0.5, mix(shop * 0.8, vec3f(1.1, 1.05, 0.95), rows), inside);
  }
  // A shutter, with some paint on it.
  let slats = 0.75 + 0.25 * smoothstep(0.3, 0.7, abs(fract(y * 12.0) - 0.5) * 2.0);
  let c = vec3f(0.3, 0.3, 0.32) * light * 2.0 * slats;
  let paint = smoothstep(0.35, 0.6, rv_fbm(vec2f(u + n * 7.0, y) * 1.3, 3)) * step(y, 1.9);
  return mix(c, hsv(h21(id + 6.6), 0.7, 0.4) * light * 3.0 + vec3f(0.004), paint * 0.6);
}

// ---- The signs on the fronts ----
// Where a real street's richness is: up one end of most fronts a stack of the tenants' signs, a
// lit box for each one above another (vertical signboards), and over many shops a big sign of one
// or two characters, a box, a disc or a diamond. Both are drawn on the front (facade()) and, near
// by, stand out from it as boxes (front_parts()), so they get their ink lines.

// The tenants' stack up one end of shop building n's front (side s): where it starts along the
// front as read from the street, its width, its foot and its top; width 0 for none. Buildings of
// three floors or more have one, the busier the block the likelier; never over a giant screen.
fn stack_of(n: f32, s: f32) -> vec4f {
  let id = vec2f(first_row(n, s), s);
  let hb = height_of(i32(s), n);
  let width = rows_of(n, s) * UNIT;
  let on = h21(id + 51.7) < mix(0.45, 0.85, busy(n)) && hb > 12.0 && front_kind(n, s) >= 0.16;
  let w = 0.8 + 0.5 * h21(id + 52.3);
  let start = select(width - 0.3 - w, 0.3, h21(id + 53.9) < 0.5);
  let top = min(hb - 0.8, 3.5 + STOREY * (2.0 + floor(h21(id + 54.1) * 4.0)));
  return select(vec4f(0.0), vec4f(start, w, 3.5, top), on);
}

// The big sign's shape on building `id`: 0 a box, 1 a disc, 2 a diamond, 3 a tall box of two
// characters.
fn big_shape(id: vec2f) -> f32 {
  return floor(h21(id + 63.7) * 4.0);
}

// The big sign over the shop of building n (side s), beside its stack `st` (stack_of): where it
// starts along the front as read from the street, its top, width and height; width 0 for none.
fn big_box_of(n: f32, s: f32, st: vec4f) -> vec4f {
  let id = vec2f(first_row(n, s), s);
  let hb = height_of(i32(s), n);
  let width = rows_of(n, s) * UNIT;
  // The front the stack leaves free.
  var free = vec2f(0.2, width - 0.2);
  if (st.y > 0.0) {
    free = select(vec2f(st.x + st.y + 0.3, width - 0.2), vec2f(0.2, st.x - 0.3), st.x > 0.5 * width);
  }
  let size = min(1.5 + 0.7 * h21(id + 62.9), free.y - free.x);
  let h = select(size, 1.85 * size, big_shape(id) > 2.5);
  let on = h21(id + 61.3) < 0.55 && front_kind(n, s) >= 0.16 && size >= 1.1 && hb > 4.6 + h;
  return select(vec4f(0.0), vec4f(0.5 * (free.x + free.y - size), 3.6 + h, size, h), on);
}

// Where along the street (z) the stretch a0..a1 of building b's front (side s, `width` long) is,
// as read from the street.
fn front_z(b: f32, width: f32, s: f32, a0: f32, a1: f32) -> vec2f {
  return select(vec2f(a0, a1), vec2f(width - a1, width - a0), s > 0.0) + b * UNIT;
}

// The stack of building b (side s): a box for each tenant, each its own colour, face and word
// down it (lettered()), now and then one or two big characters or a frame of bulbs; some dark in
// a quiet section. x across it as read from the street, y down from its top.
fn stack_face(b: f32, s: f32, x: f32, y: f32, width: f32, height: f32, aa: f32) -> vec3f {
  let count = max(floor(height / 1.7), 1.0);
  let seg = height / count;
  let k = min(floor(y / seg), count - 1.0);
  let v = y - k * seg;
  let tid = vec2f(b * 5.0 + k, s * 3.0 + 41.0);
  let word = select(word_for(tid, 4.0), big_word(tid, h21(tid + 9.7) < 0.4), h21(tid + 9.9) < 0.3);
  let frame = select(1.0, 2.0, h21(tid + 13.1) < 0.15);
  var col = lettered(tid, word, face_of(tid), true, x, v - 0.05, width, seg - 0.1, floor(h21(tid + 3.6) * 4.0), frame, 0.0, aa);
  col *= glow_now() * (0.75 + 0.4 * h21(tid + 2.8)) * flicker(tid);
  col = mix(col * 0.03 + vec3f(0.01), col, lit(tid));
  // The frame between the boxes.
  return mix(col, vec3f(0.03), 1.0 - rv_band(v, 0.05, seg - 0.05, aa));
}

// The big sign of building `id` (big_box_of): its colour and how much of the pixel it covers; x
// across from its left as read from the street, y down from its top. A disc or a diamond has its
// own edge for a rim: dark, a white line inside it.
fn big_face(id: vec2f, x: f32, y: f32, size: vec2f, aa: f32) -> vec4f {
  let shape = big_shape(id);
  let style = floor(h21(id + 3.6) * 4.0);
  let glow = glow_now() * (0.8 + 0.35 * h21(id + 2.8)) * flicker(id);
  var col: vec3f;
  var cover = 1.0;
  if (shape < 0.5 || shape > 2.5) {
    let frame = select(1.0, 2.0, h21(id + 13.1) < 0.3);
    col = lettered(id, big_word(id, shape > 2.5), face_of(id), true, x, y, size.x, size.y, style, frame, 0.0, aa) * glow;
  } else {
    let q = vec2f(x, y) - 0.5 * size;
    let r = 0.5 * size.x;
    let disc = shape < 1.5;
    let sd = select((abs(q.x) + abs(q.y) - r) * 0.7071, length(q) - r, disc);
    col = lettered(id, big_word(id, false), face_of(id), true, x, y, size.x, size.y, style, 0.0, select(0.25, 0.15, disc) * size.x, aa) * glow;
    col = mix(col, vec3f(1.6, 1.55, 1.45) * glow, smoothstep(0.022 + aa, 0.0, abs(sd + 0.1)));
    col = mix(col, vec3f(0.035), smoothstep(-0.05 - aa, -0.05, sd));
    cover = smoothstep(aa, -aa, sd);
  }
  return vec4f(mix(col * 0.03 + vec3f(0.01), col, lit(id)), cover);
}

// The front of shop building n on side s, facing the street: u metres along it, y up.
fn facade(n: f32, s: f32, u: f32, y: f32, aa: f32) -> vec3f {
  let id = vec2f(n, s);
  let hb = height_of(i32(s), n);
  let sg = sign_of(n, s);
  let near_sign = select(0.0, exp(-abs(n * UNIT + u - sg.x) * 0.9) * smoothstep(sg.z - 2.0, sg.z, y) * smoothstep(sg.w + 2.0, sg.w, y), sg.y > 0.0);
  let light = params.ambient * vec3f(0.9, 0.95, 1.1) + shop_light(n, s) * 0.12 * exp(-max(y - 2.5, 0.0) * 0.45) + sign_light(n, s) * 0.08 * near_sign;
  // As read standing in the street facing the wall: on the right side +z is on the left.
  let along = select(u, UNIT - u, s > 0.0);
  if (y < 3.1) {
    // Grime along the foot of the wall.
    let foot = 0.7 + 0.3 * smoothstep(0.0, 0.35, y);
    return shop_front(n, s, u, along, y, light, aa) * (0.3 + 0.7 * rv_band(u, 0.08, UNIT - 0.08, aa)) * foot;
  }
  // The building may span two rows: ub runs along all of it, ab as read from the street.
  let b = first_row(n, s);
  let bid = vec2f(b, s);
  let width = rows_of(n, s) * UNIT;
  let ub = n * UNIT + u - b * UNIT;
  let ab = select(ub, width - ub, s > 0.0);
  let kind = front_kind(n, s);
  var c: vec3f;
  if (kind < 0.16 && hb > 12.0) {
    // A giant screen over four floors.
    let top = min(hb - 1.2, 3.4 + 4.0 * STOREY);
    c = albedo_of(bid) * light * 0.5;
    if (y > 3.4 && y < top && ab > 0.3 && ab < width - 0.3) {
      c = big_screen(bid, ab - 0.3, top - y, width - 0.6, top - 3.4, aa);
    }
  } else if (kind < 0.32) {
    // Neon tubes up the front.
    c = storeys(bid, 0.2, ub, y - 0.1, hb, light, aa) * 0.5;
    let k = floor(ub / 0.75);
    let x = fract(ub / 0.75) - 0.5;
    let tube = smoothstep(0.06 + aa, 0.0, abs(x)) * rv_band(y, 3.3, hb - 0.8, aa) * step(0.5, k) * step(k, width / 0.75 - 1.5);
    c += neon(h21(vec2f(k, b) + 5.5)) * 3.0 * tube * lit(bid);
  } else if (kind < 0.47) {
    // Glass offices.
    c = storeys(bid, 0.65, ub, y - 0.1, hb, light, aa);
  } else if (kind < 0.6) {
    // Flats: balconies, and air conditioners under some windows.
    c = storeys(bid, 0.95, ub, y - 0.1, hb, light, aa);
    let fy = fract((y - 0.1) / STOREY);
    let ac = rv_band(fract(ub / 1.5), 0.55, 0.85, aa / 1.5) * rv_band(fy, 0.24, 0.4, aa / STOREY)
      * step(h21(vec2f(floor(ub / 1.5), floor((y - 0.1) / STOREY)) + bid), 0.5);
    c = mix(c, vec3f(0.5, 0.5, 0.48) * light * 2.0 + vec3f(0.006), ac);
  } else {
    // Windows, with panels on some of the lower floors.
    let fl = floor((y - 3.1) / STOREY);
    let fy = (y - 3.1) / STOREY - fl;
    let fid = vec2f(n * 7.0 + fl, s * 13.0 + 1.0);
    if (h21(fid) < 0.22 && fl < 3.0 && y < hb - 0.6) {
      c = panel(fid, along - 0.25, (0.88 - fy) * STOREY, UNIT - 0.5, 0.76 * STOREY, aa);
    } else {
      c = storeys(bid, h21(bid + 3.1), ub, y - 0.1, hb, light, aa);
    }
  }
  // Darker into the corners and under the fascia.
  let corner = 0.7 + 0.3 * smoothstep(0.0, 0.4, min(ub, width - ub));
  let under = 0.6 + 0.4 * smoothstep(3.1, 3.7, y);
  c *= (0.3 + 0.7 * rv_band(ub, 0.08, width - 0.08, aa)) * corner * under;
  // The big sign over the shop and the tenants' stack.
  let st = stack_of(n, s);
  let bx = big_box_of(n, s, st);
  if (bx.z > 0.0 && ab > bx.x && ab < bx.x + bx.z && y < bx.y && y > bx.y - bx.w) {
    let sign = big_face(bid, ab - bx.x, bx.y - y, bx.zw, aa);
    c = mix(c, sign.rgb, sign.a);
  }
  if (st.y > 0.0 && ab > st.x && ab < st.x + st.y && y > st.z && y < st.w) {
    c = stack_face(b, s, ab - st.x, st.w - y, st.y, st.w - st.z, aa);
  }
  return c;
}

// A roof seen from above: concrete with a parapet, machinery, now and then a red light.
fn roof(i: i32, n: f32, xz: vec2f, size: vec2f, aa: f32) -> vec3f {
  let id = vec2f(f32(i) * 7.3, n);
  var c = vec3f(0.035, 0.035, 0.04);
  let edge = min(min(xz.x, size.x - xz.x), min(xz.y, size.y - xz.y));
  c = mix(c, vec3f(0.08, 0.08, 0.09), rv_band(edge, 0.0, 0.25, aa));
  // Boxes of machinery on a coarse grid.
  let cell = floor(xz / 1.5);
  let f = fract(xz / 1.5) - 0.5;
  let box_on = step(h21(cell + id), 0.18) * step(0.6, edge);
  c = mix(c, vec3f(0.12, 0.12, 0.13), box_on * smoothstep(0.3 + aa, 0.3 - aa, max(abs(f.x), abs(f.y))));
  // A red light at a corner, blinking as an aircraft's obstruction light does: about once a second
  // on a clock of its own (its own period and phase for each roof), rising and fading softly, and
  // not with the music (no light pulses on the beat).
  let beacon = step(h21(id + 4.4), 0.3) * smoothstep(0.25 + aa, 0.0, length(xz - vec2f(0.4)));
  let period = 0.9 + 0.3 * h21(id + 6.6);
  let rise = 0.5 - 0.5 * cos(6.2832 * fract(frame.time / period + h21(id + 5.5)));
  c += vec3f(5.0, 0.4, 0.25) * beacon * (0.3 + 1.7 * rise * rise);
  return c;
}

// A wall of the city that does not face the street.
fn city_wall(i: i32, n: f32, p: vec3f, kind: f32, aa: f32) -> vec3f {
  let hb = height_of(i, n);
  let id = vec2f(f32(i) * 7.3, select(n, floor(n * 0.5), abs(i) > 1));
  let u = select(abs(p.x), p.z, kind < 1.5);
  // Lit faintly by the city's glow from below.
  let light = params.ambient * vec3f(0.9, 0.95, 1.1) + vec3f(0.025, 0.018, 0.012) * exp(-p.y * 0.08);
  return storeys(id, h21(id + 3.1), u, p.y, hb, light, aa);
}

// ---- Things over the street ----

// The face of the sign sg (as sign_of) on side s, whose wall stands at `wall`, along the ray, or
// FAR.
fn hit_sign(o: vec3f, d: vec3f, s: f32, sg: vec4f, wall: f32) -> f32 {
  let t = (sg.x - o.z) / d.z;
  let p = o + d * t;
  let a = (wall - 0.12) - s * p.x;
  if (sg.y > 0.0 && t > 0.0 && a > 0.0 && a < sg.y && p.y > sg.z && p.y < sg.w) {
    return t;
  }
  return FAR;
}

// ---- Traffic lights ----
// A Japanese traffic light: a pole at the kerb, an arm over the road, and hung from it the three
// lamps side by side in one grey housing, each under its hood (green, which Japan calls blue,
// then yellow, then red, from the left), facing the way the street is walked; on the pole the
// pedestrians' light facing across and the blue crossing sign. One stands at a corner of every
// cross street, and the burst may film one put mid-block over a crossing painted there (hero).
// The lamps change every four bars (state.signal) and are steady between.

fn signal_pole(s: f32) -> f32 {
  return s * (HALF - 0.35);
}

fn signal_head(s: f32) -> f32 {
  return s * (HALF - 2.75);
}

// The side of cross street n whose corner has the traffic light.
fn signal_side(n: f32) -> f32 {
  return select(-1.0, 1.0, h21(vec2f(n, 81.3)) < 0.5);
}

// The ray against the traffic light at z = zp on side s: t and the part (0 metal, 1 the housing,
// 2 a hood, 3 the pedestrians' light, 4 the crossing sign); t FAR for a miss.
fn signal_hit(o: vec3f, d: vec3f, zp: f32, s: f32, pix: f32) -> vec2f {
  let inv = inverse(d);
  let px = signal_pole(s);
  let hx = signal_head(s);
  let ends = vec2f(px + s * 0.12, px - s * 3.2);
  var best = vec2f(FAR, 0.0);
  // All of it is in this box: a ray that misses it meets none of it.
  if (!in_box(o, inv, vec3f(min(ends.x, ends.y), 0.0, zp - 0.3), vec3f(max(ends.x, ends.y), 5.85, zp + 0.2))) {
    return best;
  }
  let len = length(d);
  let rd = d / len;
  // Thin parts are widened to half a pixel where they are, so far off they do not break up.
  let fp = 0.5 * pix * length(vec3f(px, 3.0, zp) - o);
  var t = capsule_t(o, rd, vec3f(px, 0.0, zp), vec3f(px, 5.75, zp), 0.09 + fp);
  t = nearer(t, capsule_t(o, rd, vec3f(px, 5.55, zp), vec3f(px - s * 3.1, 5.55, zp), 0.055 + fp));
  t = nearer(t, capsule_t(o, rd, vec3f(hx, 5.55, zp), vec3f(hx, 5.33, zp), 0.03 + fp));
  if (t > 0.0) {
    best = vec2f(t / len, 0.0);
  }
  let head = box_hit(o, inv, vec3f(hx - 0.62, 4.95, zp - 0.09), vec3f(hx + 0.62, 5.35, zp + 0.09));
  if (head.x > 0.0 && head.x < best.x) {
    best = vec2f(head.x, 1.0);
  }
  for (var k = -1.0; k <= 1.0; k += 1.0) {
    let c = hx + 0.4 * k;
    let hood = box_hit(o, inv, vec3f(c - 0.17, 5.29, zp - 0.27), vec3f(c + 0.17, 5.32, zp - 0.09));
    if (hood.x > 0.0 && hood.x < best.x) {
      best = vec2f(hood.x, 2.0);
    }
  }
  let xa = px - s * 0.12;
  let xb = px - s * 0.34;
  let walk = box_hit(o, inv, vec3f(min(xa, xb), 2.3, zp - 0.18), vec3f(max(xa, xb), 3.0, zp + 0.18));
  if (walk.x > 0.0 && walk.x < best.x) {
    best = vec2f(walk.x, 3.0);
  }
  let xc = px - s * 0.06;
  let xd = px - s * 0.6;
  let plate = box_hit(o, inv, vec3f(min(xc, xd), 3.55, zp - 0.13), vec3f(max(xc, xd), 4.1, zp - 0.11));
  if (plate.x > 0.0 && plate.x < best.x) {
    best = vec2f(plate.x, 4.0);
  }
  return best;
}

fn signal_face(part: f32, s: f32, zp: f32, p: vec3f, aa: f32) -> vec3f {
  let light = street_light(vec3f(p.x, 0.0, p.z), true) * 0.3 * exp(-p.y * 0.15) + params.ambient * vec3f(0.9, 0.95, 1.1) * 3.0;
  let go = state.signal > 0.5;
  if (part < 0.5) {
    return vec3f(0.32, 0.33, 0.35) * light;
  }
  if (part < 1.5) {
    var c = vec3f(0.12, 0.13, 0.14) * light;
    if (p.z < zp - 0.085) {
      // The lamps, each in a black ring: the one on lit, the others dark in their colours.
      let q = vec2f(p.x - signal_head(s), p.y - 5.15);
      let k = clamp(round(q.x / 0.4), -1.0, 1.0);
      let r = length(vec2f(q.x - 0.4 * k, q.y));
      var lamp = select(vec3f(0.01, 0.035, 0.03), vec3f(0.15, 1.0, 0.75) * 4.5, go);
      if (k > 0.5) {
        lamp = select(vec3f(0.05, 0.004, 0.002), vec3f(6.0, 0.3, 0.1), !go);
      } else if (k > -0.5) {
        lamp = vec3f(0.05, 0.035, 0.0);
      }
      c = mix(c, vec3f(0.01), smoothstep(0.17 + aa, 0.17, r));
      c = mix(c, lamp, smoothstep(0.135 + aa, 0.135, r));
    }
    return c;
  }
  if (part < 2.5) {
    return vec3f(0.1, 0.105, 0.11) * light;
  }
  if (part < 3.5) {
    // The pedestrians' light: a red figure standing over a green one walking, the green lit while
    // the cars stop.
    var c = vec3f(0.06) * light;
    if (abs(p.x - (signal_pole(s) - s * 0.34)) < 0.01) {
      let top = p.y > 2.65;
      let f = vec2f(p.z - zp, p.y - select(2.47, 2.83, top));
      let man = min(length(f - vec2f(0.0, 0.1)) - 0.035, rv_sd_box(f + vec2f(0.0, 0.03), vec2f(0.04, 0.09)));
      let colour = select(vec3f(0.1, 1.0, 0.75) * 3.5, vec3f(5.0, 0.3, 0.12), top);
      c = mix(c, vec3f(0.01), rv_band(f.y, -0.15, 0.15, aa) * rv_band(f.x, -0.15, 0.15, aa));
      c = mix(c, select(colour * 0.015, colour, top == go), smoothstep(aa, -aa, man));
    }
    return c;
  }
  // The crossing sign: blue, a white triangle; the street's light on it.
  if (p.z < zp - 0.125) {
    let q = vec2f(p.x - (signal_pole(s) - s * 0.33), p.y - 3.825);
    let tri = step(-0.15, q.y) * step(abs(q.x), 0.19 * (0.18 - q.y) / 0.33);
    return mix(vec3f(0.03, 0.12, 0.6), vec3f(0.85), tri) * (light * 1.5 + vec3f(0.03));
  }
  return vec3f(0.3) * light;
}

// ---- The burst's subjects ----
// What the burst's detail shot puts in its row (hero_row in state.wgsl) besides the torii, the
// vending machines and the shrine the street already has: a mamachari, a traffic light (above)
// and a shopping street's lamp.

// The street lamp at z = zp on side s: a tall pole at the kerb, three frosted globes hanging from
// its crown (suzuran), lit and steady, and the shopping street's flag on two brackets.
fn lamp_pole(s: f32) -> f32 {
  return s * (HALF - 0.4);
}

// The ray against it: t and the part (0 metal, 1 a globe, 2 the flag); t FAR for a miss.
fn lamp_hit(o: vec3f, d: vec3f, zp: f32, s: f32, pix: f32) -> vec2f {
  let inv = inverse(d);
  let px = lamp_pole(s);
  let ends = vec2f(px + s * 0.2, px - s * 0.75);
  var best = vec2f(FAR, 0.0);
  if (!in_box(o, inv, vec3f(min(ends.x, ends.y), 0.0, zp - 0.72), vec3f(max(ends.x, ends.y), 4.92, zp + 0.72))) {
    return best;
  }
  let len = length(d);
  let rd = d / len;
  let fp = 0.5 * pix * length(vec3f(px, 3.0, zp) - o);
  let crown = vec3f(px, 4.72, zp);
  var t = capsule_t(o, rd, vec3f(px, 0.0, zp), crown, 0.075 + fp);
  t = nearer(t, capsule_t(o, rd, crown, vec3f(px, 4.53, zp - 0.5), 0.028 + fp));
  t = nearer(t, capsule_t(o, rd, crown, vec3f(px, 4.53, zp + 0.5), 0.028 + fp));
  t = nearer(t, capsule_t(o, rd, crown, vec3f(px - s * 0.5, 4.53, zp), 0.028 + fp));
  t = nearer(t, sphere_t(o, rd, vec3f(px, 4.82, zp), 0.07));
  t = nearer(t, capsule_t(o, rd, vec3f(px, 3.95, zp), vec3f(px - s * 0.55, 3.95, zp), 0.018 + fp));
  t = nearer(t, capsule_t(o, rd, vec3f(px, 2.55, zp), vec3f(px - s * 0.55, 2.55, zp), 0.018 + fp));
  if (t > 0.0) {
    best = vec2f(t / len, 0.0);
  }
  var g = sphere_t(o, rd, vec3f(px, 4.37, zp - 0.5), 0.16);
  g = nearer(g, sphere_t(o, rd, vec3f(px, 4.37, zp + 0.5), 0.16));
  g = nearer(g, sphere_t(o, rd, vec3f(px - s * 0.5, 4.37, zp), 0.16));
  if (g > 0.0 && g / len < best.x) {
    best = vec2f(g / len, 1.0);
  }
  let tf = (zp - o.z) / d.z;
  let q = o + d * tf;
  let a = s * (px - q.x);
  if (tf > 0.0 && tf < best.x && a > 0.07 && a < 0.53 && q.y > 2.6 && q.y < 3.9) {
    best = vec2f(tf, 2.0);
  }
  return best;
}

fn lamp_face(part: f32, s: f32, zp: f32, p: vec3f, aa: f32) -> vec3f {
  let light = street_light(vec3f(p.x, 0.0, p.z), true) * 0.3 + params.ambient * vec3f(0.9, 0.95, 1.1) * 3.0;
  if (part < 0.5) {
    return vec3f(0.1, 0.13, 0.12) * light;
  }
  if (part < 1.5) {
    return vec3f(2.6, 2.25, 1.7) * (0.75 + 0.3 * clamp((4.53 - p.y) / 0.32, 0.0, 1.0));
  }
  // The flag, lit by the globes above it: the street's name down it.
  let a = s * (lamp_pole(s) - p.x);
  let ink = vertical_word(FACE_DELA, 32.0, a - 0.16, 3.85 - p.y, 0.28, 1.2, aa, 0.0);
  return mix(vec3f(0.55, 0.05, 0.04), vec3f(0.9, 0.88, 0.82), ink) * (light * 2.0 + vec3f(0.25, 0.2, 0.15));
}

// The burst's lamp lights what is round it (nothing while there is none).
fn hero_light(p: vec3f) -> vec3f {
  if (state.hero.w < 0.5 || state.hero.x != HERO_LAMP) {
    return vec3f(0.0);
  }
  let lp = vec3f(lamp_pole(state.hero.z) - state.hero.z * 0.2, 4.37, state.hero.y * UNIT + 2.25);
  let dl = lp - p;
  return vec3f(1.0, 0.85, 0.62) * 2.5 * max(lp.y - p.y, 0.3) / pow(dot(dl, dl) + 0.5, 1.5);
}

// The burst's bicycle: a mamachari parked square to the wall before shop (n, s), its front wheel
// and its basket to the road: two spoked wheels, a step-through frame, swept-back bars, a saddle,
// a carrier, a chain case and a wire basket. In its own frame u runs from the back axle towards
// the road, y up, v along the street; b0 is the back axle's foot.
fn bike_at(b0: vec3f, s: f32, q: vec3f) -> vec3f {
  return b0 + vec3f(-s * q.x, q.y, q.z);
}

fn bike_box(o: vec3f, inv: vec3f, b0: vec3f, s: f32, lo: vec3f, hi: vec3f) -> f32 {
  let a = bike_at(b0, s, lo);
  let b = bike_at(b0, s, hi);
  let r = box_hit(o, inv, min(a, b), max(a, b));
  return select(-1.0, r.x, r.x > 0.0);
}

// The ray against it: t and the part (0 the painted frame, 1 a tyre, 2 steel: rims, spokes, bars,
// the carrier and the basket, 3 the saddle, 4 the chain case); t FAR for a miss.
fn mamachari_hit(o: vec3f, d: vec3f, n: f32, s: f32, pix: f32) -> vec2f {
  let inv = inverse(d);
  let b0 = vec3f(s * (wall_x(n, s) - 0.55), 0.0, n * UNIT + 2.25);
  var best = vec2f(FAR, 0.0);
  let lo = bike_at(b0, s, vec3f(-0.4, 0.0, -0.33));
  let hi = bike_at(b0, s, vec3f(1.4, 1.12, 0.33));
  if (!in_box(o, inv, min(lo, hi), max(lo, hi))) {
    return best;
  }
  let len = length(d);
  let rd = d / len;
  let fp = 0.5 * pix * length(bike_at(b0, s, vec3f(0.5, 0.6, 0.0)) - o);
  // The frame.
  var t = capsule_t(o, rd, bike_at(b0, s, vec3f(0.0, 0.33, 0.0)), bike_at(b0, s, vec3f(0.45, 0.27, 0.0)), 0.018 + fp);
  t = nearer(t, capsule_t(o, rd, bike_at(b0, s, vec3f(0.0, 0.33, 0.0)), bike_at(b0, s, vec3f(0.3, 0.8, 0.0)), 0.015 + fp));
  t = nearer(t, capsule_t(o, rd, bike_at(b0, s, vec3f(0.45, 0.27, 0.0)), bike_at(b0, s, vec3f(0.28, 0.86, 0.0)), 0.022 + fp));
  t = nearer(t, capsule_t(o, rd, bike_at(b0, s, vec3f(0.45, 0.27, 0.0)), bike_at(b0, s, vec3f(0.62, 0.42, 0.0)), 0.026 + fp));
  t = nearer(t, capsule_t(o, rd, bike_at(b0, s, vec3f(0.62, 0.42, 0.0)), bike_at(b0, s, vec3f(0.86, 0.82, 0.0)), 0.026 + fp));
  t = nearer(t, capsule_t(o, rd, bike_at(b0, s, vec3f(0.86, 0.8, 0.0)), bike_at(b0, s, vec3f(0.88, 0.93, 0.0)), 0.03 + fp));
  t = nearer(t, capsule_t(o, rd, bike_at(b0, s, vec3f(0.88, 0.82, 0.0)), bike_at(b0, s, vec3f(1.05, 0.33, 0.0)), 0.02 + fp));
  if (t > 0.0) {
    best = vec2f(t / len, 0.0);
  }
  // Steel: the stem, the bars swept back to their grips, the seat post.
  var st = capsule_t(o, rd, bike_at(b0, s, vec3f(0.88, 0.92, 0.0)), bike_at(b0, s, vec3f(0.84, 1.04, 0.0)), 0.016 + fp);
  st = nearer(st, capsule_t(o, rd, bike_at(b0, s, vec3f(0.84, 1.04, 0.0)), bike_at(b0, s, vec3f(0.68, 1.02, 0.27)), 0.014 + fp));
  st = nearer(st, capsule_t(o, rd, bike_at(b0, s, vec3f(0.84, 1.04, 0.0)), bike_at(b0, s, vec3f(0.68, 1.02, -0.27)), 0.014 + fp));
  st = nearer(st, capsule_t(o, rd, bike_at(b0, s, vec3f(0.28, 0.86, 0.0)), bike_at(b0, s, vec3f(0.27, 0.95, 0.0)), 0.014 + fp));
  if (st > 0.0 && st / len < best.x) {
    best = vec2f(st / len, 2.0);
  }
  // The wheels, in the plane v = 0: tyres, rims, eighteen spokes each, hubs.
  let tw = (b0.z - o.z) / d.z;
  if (tw > 0.0 && tw < best.x) {
    let q = o + d * tw;
    let u = s * (b0.x - q.x);
    let fw = 0.5 * pix * tw * len;
    for (var k = 0.0; k < 2.0; k += 1.0) {
      let c = vec2f(u - 1.05 * k, q.y - 0.33);
      let r = length(c);
      let a = (fract(atan2(c.y, c.x) / 0.349066 + 0.5) - 0.5) * 0.349066;
      if (r > 0.295 - fw && r < 0.335 + fw) {
        best = vec2f(tw, 1.0);
      } else if (r < 0.295 && (r > 0.275 - fw || r < 0.04 + fw || r * abs(a) < 0.003 + fw)) {
        best = vec2f(tw, 2.0);
      }
    }
  }
  let saddle = bike_box(o, inv, b0, s, vec3f(0.17, 0.94, -0.09), vec3f(0.38, 1.0, 0.09));
  if (saddle > 0.0 && saddle < best.x) {
    best = vec2f(saddle, 3.0);
  }
  let basket = bike_box(o, inv, b0, s, vec3f(0.93, 0.8, -0.18), vec3f(1.27, 1.04, 0.18));
  if (basket > 0.0 && basket < best.x) {
    best = vec2f(basket, 2.0);
  }
  let carrier = bike_box(o, inv, b0, s, vec3f(-0.12, 0.8, -0.1), vec3f(0.24, 0.83, 0.1));
  if (carrier > 0.0 && carrier < best.x) {
    best = vec2f(carrier, 2.0);
  }
  let case_box = bike_box(o, inv, b0, s, vec3f(0.0, 0.2, 0.03), vec3f(0.5, 0.4, 0.06));
  if (case_box > 0.0 && case_box < best.x) {
    best = vec2f(case_box, 4.0);
  }
  return best;
}

fn mamachari_face(part: f32, n: f32, s: f32, p: vec3f, aa: f32) -> vec3f {
  let light = street_light(vec3f(p.x, 0.0, p.z), true) * 0.35 + params.ambient * vec3f(0.9, 0.95, 1.1) * 3.0;
  var paints = array<vec3f, 4>(vec3f(0.75, 0.7, 0.6), vec3f(0.3, 0.55, 0.48), vec3f(0.6, 0.07, 0.05), vec3f(0.05));
  let paint = paints[min(u32(h21(vec2f(n, s) + 7.7) * 4.0), 3u)];
  if (part < 0.5) {
    return paint * light * 1.2;
  }
  if (part < 1.5) {
    return vec3f(0.02) * light;
  }
  if (part < 2.5) {
    // Steel; on the basket a mesh of wires with the dark between them.
    let steel = vec3f(0.5, 0.52, 0.55) * light;
    let u = s * (s * (wall_x(n, s) - 0.55) - p.x);
    if (u > 0.92 && p.y > 0.79) {
      let g = vec2f(select(p.z, u, abs(p.z - (n * UNIT + 2.25)) > 0.175), p.y) / 0.045;
      let wire = max(rv_band(fract(g.x), 0.0, 0.22, aa / 0.045), rv_band(fract(g.y), 0.0, 0.22, aa / 0.045));
      return mix(vec3f(0.012), steel, max(wire, smoothstep(0.3, 0.6, aa / 0.045)));
    }
    return steel;
  }
  if (part < 3.5) {
    return vec3f(0.04, 0.03, 0.025) * light;
  }
  return paint * 0.8 * light;
}

// The burst's subject in row n (hero_row) along the ray: t and the part; t FAR for none here.
fn hero_hit(o: vec3f, d: vec3f, n: f32, pix: f32) -> vec2f {
  let s = state.hero.z;
  let kind = state.hero.x;
  if (kind == HERO_BIKE) {
    return mamachari_hit(o, d, n, s, pix);
  }
  if (kind == HERO_SIGNAL) {
    return signal_hit(o, d, n * UNIT + 1.2, s, pix);
  }
  if (kind == HERO_LAMP) {
    return lamp_hit(o, d, n * UNIT + 2.25, s, pix);
  }
  return vec2f(FAR, 0.0);
}

fn hero_face(n: f32, s: f32, part: f32, p: vec3f, aa: f32) -> vec3f {
  let kind = state.hero.x;
  if (kind == HERO_BIKE) {
    return mamachari_face(part, n, s, p, aa);
  }
  if (kind == HERO_SIGNAL) {
    return signal_face(part, s, n * UNIT + 1.2, p, aa);
  }
  return lamp_face(part, s, n * UNIT + 2.25, p, aa);
}

// ---- Poles and wires ----

fn pole_x(s: f32) -> f32 {
  return s * (HALF - 0.3);
}

// The pole of row n on side s along the ray, or FAR: the pole, its cross-arm, and on some a
// transformer.
fn hit_pole(o: vec3f, d: vec3f, n: f32, s: f32) -> f32 {
  let t = (n * UNIT + 0.35 - o.z) / d.z;
  let p = o + d * t;
  let x = p.x - pole_x(s);
  let pole = abs(x) < 0.13 && p.y > 0.0 && p.y < 9.5;
  let arm = abs(x) < 0.85 && abs(p.y - 8.7) < 0.06;
  let box = h21(vec2f(n, s) + 6.6) < 0.4 && abs(x + s * 0.32) < 0.22 && p.y > 6.2 && p.y < 7.0;
  if (t > 0.0 && (pole || arm || box)) {
    return t;
  }
  return FAR;
}

fn pole_face(p: vec3f, s: f32) -> vec3f {
  // Concrete, lit from the street below and darker up high, with a band of posters.
  let light = street_light(vec3f(p.x, 0.0, p.z), true) * 0.5 * exp(-p.y * 0.12) + params.ambient * vec3f(0.9, 0.95, 1.1);
  var albedo = vec3f(0.35, 0.34, 0.32);
  albedo = mix(albedo, vec3f(0.6, 0.55, 0.4), rv_band(p.y, 1.4, 2.2, 0.01) * 0.6);
  return albedo * light;
}

// Wires running along the street from pole to pole, sagging between them: how much of the pixel
// they cover before tmax.
fn wires(o: vec3f, d: vec3f, tmax: f32, pix: f32) -> f32 {
  let len = length(d);
  let dxy = d.xy;
  let dd = dot(dxy, dxy);
  if (dd < 1e-6) {
    return 0.0;
  }
  var cover = 0.0;
  for (var k = 0.0; k < 6.0; k += 1.0) {
    let s = select(-1.0, 1.0, k < 3.0);
    let j = k - 3.0 * floor(k / 3.0);
    let x0 = pole_x(s) + s * (0.35 * j - 0.35);
    let y0 = 7.6 + 0.45 * j;
    var t = dot(vec2f(x0, y0) - o.xy, dxy) / dd;
    var p = o + d * t;
    let f = fract((p.z - 0.35) / (2.0 * UNIT));
    let y = y0 - 0.5 * (1.0 - pow(2.0 * f - 1.0, 2.0));
    t = dot(vec2f(x0, y) - o.xy, dxy) / dd;
    p = o + d * t;
    if (t <= 0.0 || t > tmax) {
      continue;
    }
    let footprint = t * len * pix;
    let r = 0.015;
    let dist = length(p.xy - vec2f(x0, y));
    // (Passing further off covers none of the pixel: the hash below is not needed then.)
    if (dist >= r + footprint || lantern_block(floor(p.z / UNIT))) {
      continue;
    }
    cover = max(cover, smoothstep(r + footprint, 0.0, dist) * min(2.0 * r / (r + footprint), 1.0));
  }
  return cover;
}

// ---- People ----

// Distance to a walking figure, q from between its feet, metres. `build` widens the coat.
fn figure(q: vec2f, height: f32, phase: f32, build: f32) -> f32 {
  let k = height / 1.7;
  let head = length((q - vec2f(0.0, 1.585 * k)) * vec2f(1.0, 0.9)) - 0.1 * k;
  let neck = rv_sd_box(q - vec2f(0.0, 1.47 * k), vec2f(0.04, 0.05) * k);
  // A coat narrowing from the shoulders and flaring a little to the hem.
  let top = 1.43 * k;
  let hem = (0.62 - 0.12 * build) * k;
  let t = clamp((top - q.y) / (top - hem), 0.0, 1.0);
  let half_width = mix(0.2, 0.15 + 0.06 * build, t) * k;
  let coat = max(abs(q.x) - half_width, max(q.y - top, hem - q.y));
  let shoulders = length((q - vec2f(0.0, 1.36 * k)) / vec2f(0.21, 0.09)) * 0.09 * k - 0.09 * k;
  let swing = sin(phase) * 0.11 * k;
  let leg1 = rv_sd_segment(q, vec2f(-0.06 * k, hem + 0.05), vec2f(-0.06 * k + swing, 0.03)) - 0.05 * k;
  let leg2 = rv_sd_segment(q, vec2f(0.06 * k, hem + 0.05), vec2f(0.06 * k - swing, 0.03)) - 0.05 * k;
  let arm1 = rv_sd_segment(q, vec2f(-0.19 * k, 1.34 * k), vec2f(-0.2 * k - swing * 0.4, 0.84 * k)) - 0.045 * k;
  let arm2 = rv_sd_segment(q, vec2f(0.19 * k, 1.34 * k), vec2f(0.2 * k + swing * 0.4, 0.84 * k)) - 0.045 * k;
  return min(min(min(head, neck), min(coat, shoulders)), min(min(leg1, leg2), min(arm1, arm2)));
}

// Person m of stream k (`who`, as person(); height 0 for nobody) along the ray, or FAR.
fn hit_person(o: vec3f, d: vec3f, who: vec4f, m: f32, k: f32, pix: f32) -> f32 {
  if (who.z <= 0.0) {
    return FAR;
  }
  let t = (who.y - o.z) / d.z;
  if (t <= 0.0) {
    return FAR;
  }
  let p = o + d * t;
  let q = vec2f(p.x - who.x, p.y);
  if (abs(q.x) > 0.4 || q.y > who.z + 0.05 || q.y < 0.0) {
    return FAR;
  }
  if (figure(q, who.z, who.w, h21(vec2f(m, k) + 9.1)) < 0.5 * t * length(d) * pix) {
    return t;
  }
  return FAR;
}

fn person_face(m: f32, k: f32, p: vec3f, d: vec3f, pix: f32) -> vec3f {
  let who = person(m, k);
  let id = vec2f(m, k);
  let q = vec2f(p.x - who.x, p.y);
  let sd = figure(q, who.z, who.w, h21(id + 9.1));
  // Dark clothes, a little of the street's light on them, more at the edge.
  let clothes = mix(vec3f(0.02, 0.02, 0.024), vec3f(0.07, 0.055, 0.045), h21(id + 8.8));
  let light = street_light(vec3f(p.x, 0.0, p.z), true) * 0.3 + params.ambient * vec3f(0.9, 0.95, 1.1);
  let rim = smoothstep(-0.025, 0.0, sd) * smoothstep(0.6, 1.4, q.y);
  return clothes * light + light * rim * 0.25;
}

fn awning_face(n: f32, s: f32, p: vec3f, axis: f32, d: vec3f) -> vec3f {
  let id = vec2f(n, s);
  let canvas = mix(neon(h21(id + 7.7)), vec3f(0.9), 0.25);
  let stripe = step(0.5, fract((p.z - n * UNIT) / 0.45));
  let cloth = mix(canvas, vec3f(0.85, 0.82, 0.78), stripe * 0.7);
  let shop = shop_light(n, s);
  if (axis > 2.5) {
    // Underneath it glows with the shop; on top it is dark.
    return select(cloth * 0.03, cloth * shop * 0.6 + cloth * 0.01, d.y > 0.0);
  }
  return cloth * (0.04 + 0.3 * rv_luma(shop));
}

// The signs, traffic lights, awnings, poles and people along the ray before `best`:
// (distance, kind, row, side or number); kind 4 a sign, 5 a traffic light, 6..8 an awning hit
// across, along or from below, 9 a pole, 10 a person.
// ---- What stands out from the fronts ----
// The fronts along the street are not flat: flats have a balcony on every floor (a slab and a
// frosted parapet), window buildings a hood over every floor's windows, glass offices a row of
// fins, giant screens a frame. The ray against those of shop building (n, s) within row n, for
// the floors between heights y0 and y1; `front` is the building's first row, rows, height and
// kind (as State's row_front), w its wall. t and the part (0 a slab or hood, 1 a parapet, 2 a fin,
// 3 a frame) with the face it meets (as box_hit) in tenths; t FAR for a miss.
fn front_parts(o: vec3f, inv: vec3f, n: f32, s: f32, front: vec4f, w: f32, y0: f32, y1: f32) -> vec2f {
  var best = vec2f(FAR, 0.0);
  let b = front.x;
  let width = front.y * UNIT;
  let hb = front.z;
  let kind = front.w;
  // This row's share of the building's length.
  let za = max(n * UNIT, b * UNIT + 0.2);
  let zb = min(n * UNIT + UNIT, b * UNIT + width - 0.2);
  // The tenants' stack (0.3 m deep) and the big sign over the shop (0.22 m; a disc or a diamond
  // stays flat on the front), part 4: their faces in tenths, the big sign's 5 more.
  let st = stack_of(n, s);
  if (st.y > 0.0 && y1 > st.z && y0 < st.w) {
    let z = front_z(b, width, s, st.x, st.x + st.y);
    let zr = vec2f(max(z.x, n * UNIT), min(z.y, n * UNIT + UNIT));
    if (zr.y > zr.x) {
      let xa = s * w;
      let xb = s * (w - 0.3);
      let r = box_hit(o, inv, vec3f(min(xa, xb), st.z, zr.x), vec3f(max(xa, xb), st.w, zr.y));
      if (r.x > 0.0 && r.x < best.x) {
        best = vec2f(r.x, 4.0 + 0.1 * r.y);
      }
    }
  }
  let bx = big_box_of(n, s, st);
  let shape = big_shape(vec2f(b, s));
  if (bx.z > 0.0 && (shape < 0.5 || shape > 2.5) && y1 > bx.y - bx.w && y0 < bx.y) {
    let z = front_z(b, width, s, bx.x, bx.x + bx.z);
    let zr = vec2f(max(z.x, n * UNIT), min(z.y, n * UNIT + UNIT));
    if (zr.y > zr.x) {
      let xa = s * w;
      let xb = s * (w - 0.22);
      let r = box_hit(o, inv, vec3f(min(xa, xb), bx.y - bx.w, zr.x), vec3f(max(xa, xb), bx.y, zr.y));
      if (r.x > 0.0 && r.x < best.x) {
        best = vec2f(r.x, 4.0 + 0.1 * (r.y + 5.0));
      }
    }
  }
  if (kind < 0.16) {
    if (hb <= 12.0) {
      return best;
    }
    // The giant screen's frame.
    let top = min(hb - 1.2, 3.4 + 4.0 * STOREY);
    for (var k = 0; k < 4; k++) {
      var y = vec2f(3.15, 3.4);
      var z = vec2f(za, zb);
      if (k == 1) {
        y = vec2f(top, top + 0.25);
      } else if (k == 2) {
        y = vec2f(3.15, top + 0.25);
        z = vec2f(b * UNIT + 0.05, b * UNIT + 0.3);
      } else if (k == 3) {
        y = vec2f(3.15, top + 0.25);
        z = vec2f(b * UNIT + width - 0.3, b * UNIT + width - 0.05);
      }
      z = vec2f(max(z.x, n * UNIT), min(z.y, n * UNIT + UNIT));
      if (z.y <= z.x) {
        continue;
      }
      let xa = s * w;
      let xb = s * (w - 0.35);
      let r = box_hit(o, inv, vec3f(min(xa, xb), y.x, z.x), vec3f(max(xa, xb), y.y, z.y));
      if (r.x > 0.0 && r.x < best.x) {
        best = vec2f(r.x, 3.0 + 0.1 * r.y);
      }
    }
    return best;
  }
  if (kind < 0.32) {
    return best;
  }
  if (kind < 0.47) {
    // Glass offices: fins every 1.2 m up the whole front.
    let first = ceil((za - b * UNIT - 0.3) / 1.2);
    for (var k = 0.0; k < 5.0; k += 1.0) {
      let zf = b * UNIT + 0.3 + (first + k) * 1.2;
      if (zf > zb) {
        break;
      }
      let xa = s * w;
      let xb = s * (w - 0.3);
      let r = box_hit(o, inv, vec3f(min(xa, xb), 3.3, zf - 0.03), vec3f(max(xa, xb), hb - 0.6, zf + 0.03));
      if (r.x > 0.0 && r.x < best.x) {
        best = vec2f(r.x, 2.0 + 0.1 * r.y);
      }
    }
    return best;
  }
  // Flats and window buildings: a balcony or a hood on every floor the ray's height takes in.
  let flats = kind < 0.6;
  let deep = select(0.45, 0.95, flats);
  let f0 = max(floor((y0 - 3.3) / STOREY) - 1.0, 0.0);
  let f1 = floor((y1 - 3.3) / STOREY);
  for (var f = f0; f <= f1 && f < f0 + 6.0; f += 1.0) {
    let yb = 3.3 + f * STOREY;
    if (yb + STOREY > hb - 0.6) {
      break;
    }
    let xa = s * w;
    let xb = s * (w - deep);
    let slab_y = select(vec2f(yb + 2.6, yb + 2.7), vec2f(yb, yb + 0.15), flats);
    let r = box_hit(o, inv, vec3f(min(xa, xb), slab_y.x, za), vec3f(max(xa, xb), slab_y.y, zb));
    if (r.x > 0.0 && r.x < best.x) {
      best = vec2f(r.x, 0.1 * r.y);
    }
    if (flats) {
      let xc = s * (w - deep + 0.05);
      let rp = box_hit(o, inv, vec3f(min(xb, xc), yb + 0.15, za), vec3f(max(xb, xc), yb + 1.1, zb));
      if (rp.x > 0.0 && rp.x < best.x) {
        best = vec2f(rp.x, 1.0 + 0.1 * rp.y);
      }
    }
  }
  return best;
}

fn front_part_face(n: f32, s: f32, part: f32, p: vec3f, dn: vec3f, aa: f32) -> vec3f {
  let piece = floor(part + 1e-3);
  let face = round(fract(part) * 10.0);
  let bid = vec2f(first_row(n, s), s);
  let below = shop_colour(n, s) * 0.12 * exp(-max(p.y - 2.5, 0.0) * 0.45);
  let ambient = params.ambient * vec3f(0.9, 0.95, 1.1);
  let sky = params.ambient * vec3f(0.75, 0.85, 1.25) * 3.0;
  // Tops catch the sky, undersides the shops' light below, fronts a little of both.
  var light = ambient + below * 0.6 + sky * 0.25;
  if (face > 2.5) {
    light = select(ambient * 0.5 + below * 1.4, ambient + sky, dn.y < 0.0);
  }
  if (piece > 3.5) {
    // The stack or the big sign: its face to the street as the front draws it, its sides dark.
    let big = face > 5.5;
    let depth = select(0.3, 0.22, big);
    let w = wall_x(n, s);
    if (!(abs(face - select(1.0, 6.0, big)) < 0.5 && abs(s * p.x - (w - depth)) < 0.02)) {
      return vec3f(0.03) * light;
    }
    let width = rows_of(n, s) * UNIT;
    let ub = p.z - bid.x * UNIT;
    let ab = select(ub, width - ub, s > 0.0);
    let st = stack_of(n, s);
    if (big) {
      let bx = big_box_of(n, s, st);
      return big_face(bid, ab - bx.x, bx.y - p.y, bx.zw, aa).rgb;
    }
    return stack_face(bid.x, s, ab - st.x, st.w - p.y, st.y, st.w - st.z, aa);
  }
  if (piece > 2.5) {
    return vec3f(0.06, 0.06, 0.07) * light;
  }
  if (piece > 1.5) {
    return vec3f(0.18, 0.2, 0.24) * light;
  }
  if (piece > 0.5) {
    // A frosted parapet in a frame, its posts every 0.9 m.
    let post = rv_band(fract(p.z / 0.9), 0.0, 0.06, aa / 0.9);
    let rim = 1.0 - rv_band(fract((p.y - 3.3) / STOREY) * STOREY, 0.2, 1.05, aa);
    return mix(vec3f(0.35, 0.38, 0.42), albedo_of(bid) * 1.2, max(post, rim)) * light;
  }
  return albedo_of(bid) * 1.15 * light;
}

// Vending machines against the wall of shop (n, s): two or three side by side (vendings_of, in
// state.wgsl), their fronts to the road, lit from within: rows of bottles behind glass, buttons,
// the dark slot below.

fn vending_hit(o: vec3f, inv: vec3f, n: f32, s: f32, k: f32) -> f32 {
  let w = wall_x(n, s);
  let z0 = n * UNIT + 0.5 + k * 0.95;
  let xa = s * (w - 0.05);
  let xb = s * (w - 0.75);
  let r = box_hit(o, inv, vec3f(min(xa, xb), 0.0, z0), vec3f(max(xa, xb), 1.85, z0 + 0.9));
  return select(FAR, r.x, r.x > 0.0);
}

fn vending_face(n: f32, s: f32, k: f32, p: vec3f, aa: f32) -> vec3f {
  let id = vec2f(n * 3.0 + k, s);
  var bodies = array<vec3f, 4>(vec3f(0.55, 0.05, 0.04), vec3f(0.75, 0.75, 0.72), vec3f(0.05, 0.15, 0.45), vec3f(0.08, 0.3, 0.12));
  let body = bodies[min(u32(h21(id + 98.3) * 4.0), 3u)];
  let light = street_light(vec3f(p.x, 0.0, p.z), true) * 0.25 + params.ambient * vec3f(0.9, 0.95, 1.1) * 2.0;
  let w = wall_x(n, s);
  let front = abs(p.x - s * (w - 0.75)) < 0.01;
  if (!front) {
    return body * light;
  }
  let z0 = n * UNIT + 0.5 + k * 0.95;
  let u = p.z - z0;
  var c = body * light;
  // The glass with three rows of bottles, lit white from behind.
  let glass = rv_band(u, 0.08, 0.82, aa) * rv_band(p.y, 0.95, 1.75, aa);
  if (glass > 0.0) {
    let row = floor((p.y - 0.95) / 0.27);
    let fy = fract((p.y - 0.95) / 0.27);
    let col = floor((u - 0.08) / 0.09);
    let fx = fract((u - 0.08) / 0.09);
    let bottle = rv_band(fx, 0.2, 0.8, aa / 0.09) * rv_band(fy, 0.12, 0.7, aa / 0.27);
    var hues = array<vec3f, 5>(vec3f(0.9, 0.15, 0.1), vec3f(0.95, 0.75, 0.1), vec3f(0.15, 0.4, 0.9), vec3f(0.2, 0.75, 0.3), vec3f(0.9, 0.9, 0.85));
    let drink = hues[min(u32(h21(vec2f(col, row) + id) * 5.0), 4u)];
    c = mix(c, mix(vec3f(1.1, 1.1, 1.05), drink, bottle), glass);
  }
  // A strip of lit buttons, and the dark slot at the foot.
  c = mix(c, vec3f(0.9, 0.5, 0.2) * 1.2, rv_band(u, 0.1, 0.8, aa) * rv_band(p.y, 0.88, 0.92, aa) * step(0.5, fract(u / 0.09)));
  c = mix(c, vec3f(0.01), rv_band(u, 0.15, 0.75, aa) * rv_band(p.y, 0.2, 0.45, aa));
  return c;
}

// ---- The street's furniture ----
// What makes it a Japanese shopping street at eye level: banners (nobori) on poles before the
// shops, flapping with the beat; standing boards; rows of bicycles; now and then a small
// roadside shrine against a wall, its torii facing the road; and over the street at the start of
// most blocks, the shopping street's arch with its name.

// Banner j (0 or 1) of shop (n, s): on its pole at the street's edge, the cloth hanging from the
// pole towards the middle and turning on it with the beat's swing. The ray against it: t, which
// part (0 the cloth, 1 the pole) and how far across the cloth from the pole; t FAR for a miss.
fn banner_hit(o: vec3f, d: vec3f, n: f32, s: f32, j: f32) -> vec3f {
  let pole = vec3f(s * (HALF - 0.3), 0.0, n * UNIT + 0.9 + 1.7 * j);
  var best = vec3f(FAR, 0.0, 0.0);
  let turn = state.sway * mix(0.9, 1.5, h21(vec2f(n * 2.0 + j, s) + 3.3));
  let u = vec3f(-s * cos(turn), 0.0, sin(turn));
  let nrm = vec3f(-u.z, 0.0, u.x);
  let den = dot(d, nrm);
  if (abs(den) > 1e-5) {
    let t = dot(pole - o, nrm) / den;
    let p = o + d * t;
    let a = dot(p - pole, u);
    if (t > 0.0 && a > 0.03 && a < 0.48 && p.y > 0.55 && p.y < 2.05) {
      best = vec3f(t, 0.0, a);
    }
  }
  // The pole: a thin post, met where the ray passes nearest it.
  let dd = dot(d.xz, d.xz);
  if (dd > 1e-6) {
    let t = dot(pole.xz - o.xz, d.xz) / dd;
    let p = o + d * t;
    if (t > 0.0 && t < best.x && length(p.xz - pole.xz) < 0.025 && p.y < 2.2) {
      best = vec3f(t, 1.0, 0.0);
    }
  }
  return best;
}

fn banner_face(n: f32, s: f32, j: f32, a: f32, p: vec3f, aa: f32, pole: bool) -> vec3f {
  let light = shop_colour(n, s) * 0.5 + params.ambient * vec3f(0.9, 0.95, 1.1) * 2.0;
  if (pole) {
    return vec3f(0.3) * light;
  }
  let id = vec2f(n * 2.0 + j, s);
  var cloths = array<vec3f, 5>(vec3f(0.6, 0.05, 0.04), vec3f(0.8, 0.78, 0.72), vec3f(0.85, 0.62, 0.05), vec3f(0.04, 0.1, 0.42), vec3f(0.03, 0.03, 0.03));
  let pick = u32(h21(id + 4.1) * 5.0);
  let cloth = cloths[pick];
  let ink = select(vec3f(0.95, 0.93, 0.88), vec3f(0.05, 0.03, 0.03), pick == 1u || pick == 2u);
  var words = array<f32, 5>(11.0, 37.0, 38.0, 39.0, 40.0);
  let w = words[u32(h21(id + 5.7) * 5.0)];
  // (Read from the street: on the right side the cloth reaches leftwards from its pole.)
  let x = select(a - 0.05, 0.43 - a, s > 0.0);
  let letters = vertical_word(FACE_NOTO, w, x, 1.95 - p.y, 0.38, 1.3, aa, 0.0);
  // Ripples running across it as it flaps.
  let flap = 0.85 + 0.15 * sin(a * 18.0 - frame.time * 4.0 + j * 2.0) * (0.3 + 2.0 * abs(state.sway));
  return mix(cloth, ink, letters) * light * flap;
}

// The roadside shrine before shop (n, s): a stone base and a little wooden shrine against the
// wall, a small torii before it facing the road (its beams along the street), a paper lantern
// hanging from each end of its kasagi. The ray against it: t and the part (0 the base, 1 the
// shrine, 2 its roof, 3 the torii, 4 its kasagi, 5 a lantern).
fn shrine_hit(o: vec3f, d: vec3f, inv: vec3f, n: f32, s: f32) -> vec2f {
  let w = s * wall_x(n, s);
  let zc = n * UNIT + 2.25;
  var best = vec2f(FAR, 0.0);
  // Pieces as (x from the wall towards the road: near, far), (z half), (y bottom, top).
  for (var k = 0; k < 8; k++) {
    var x = vec2f(0.05, 0.95);
    var hz = 0.5;
    var y = vec2f(0.0, 0.5);
    var part = 0.0;
    if (k == 1) {
      x = vec2f(0.15, 0.8);
      hz = 0.38;
      y = vec2f(0.5, 1.25);
      part = 1.0;
    } else if (k == 2) {
      x = vec2f(0.05, 0.92);
      hz = 0.5;
      y = vec2f(1.25, 1.42);
      part = 2.0;
    } else if (k == 3 || k == 4) {
      // The torii's pillars.
      x = vec2f(1.47, 1.55);
      hz = 0.035;
      y = vec2f(0.0, 1.5);
      part = 3.0;
    } else if (k == 5) {
      x = vec2f(1.49, 1.53);
      hz = 0.55;
      y = vec2f(1.15, 1.21);
      part = 3.0;
    } else if (k == 6) {
      x = vec2f(1.46, 1.56);
      hz = 0.68;
      y = vec2f(1.42, 1.5);
      part = 4.0;
    } else if (k == 7) {
      break;
    }
    var z0 = zc - hz;
    var z1 = zc + hz;
    if (k == 3) {
      z0 = zc - 0.48;
      z1 = zc - 0.41;
    } else if (k == 4) {
      z0 = zc + 0.41;
      z1 = zc + 0.48;
    }
    let xa = w - s * x.x;
    let xb = w - s * x.y;
    let r = box_hit(o, inv, vec3f(min(xa, xb), y.x, z0), vec3f(max(xa, xb), y.y, z1));
    if (r.x > 0.0 && r.x < best.x) {
      best = vec2f(r.x, part + 0.1 * r.y);
    }
  }
  // Two paper lanterns hanging from the kasagi's ends.
  let xl = w - s * 1.51;
  var tl = ellipsoid_t(o, d, vec3f(xl, 1.17, zc - 0.6), vec3f(0.09, 0.13, 0.09));
  tl = nearer(tl, ellipsoid_t(o, d, vec3f(xl, 1.17, zc + 0.6), vec3f(0.09, 0.13, 0.09)));
  if (tl > 0.0 && tl < best.x) {
    best = vec2f(tl, 5.0);
  }
  return best;
}

fn shrine_face(n: f32, s: f32, part: f32, p: vec3f) -> vec3f {
  let light = street_light(vec3f(p.x, 0.0, p.z), true) * 0.4 + params.ambient * vec3f(0.9, 0.95, 1.1) * 2.0;
  let piece = floor(part);
  let face = round(fract(part) * 10.0);
  if (piece > 4.5) {
    // A paper lantern lit red from within, steady: ribs round it, black at its top and foot.
    let v = (p.y - 1.17) / 0.13;
    let ribs = 0.75 + 0.25 * cos(v * 18.0);
    return select(vec3f(2.2, 0.35, 0.12) * ribs * (1.1 - 0.5 * v * v), vec3f(0.02), abs(v) > 0.8);
  }
  if (piece > 3.5) {
    // The kasagi, black on top.
    return select(vec3f(0.55, 0.06, 0.025), vec3f(0.02), face > 2.5) * light;
  }
  if (piece > 2.5) {
    return vec3f(0.55, 0.06, 0.025) * light;
  }
  if (piece > 1.5) {
    return vec3f(0.05, 0.05, 0.055) * light;
  }
  if (piece > 0.5) {
    // Old wood; a candle's light through the open front facing the road.
    let w = s * wall_x(n, s);
    let front = face < 1.5 && abs(p.x - (w - s * 0.8)) < 0.02;
    let glow = select(0.0, 1.0, front) * smoothstep(0.25, 0.0, abs(p.z - (n * UNIT + 2.25))) * smoothstep(0.3, 0.0, abs(p.y - 0.85));
    return vec3f(0.22, 0.13, 0.07) * light + vec3f(1.0, 0.55, 0.2) * 1.5 * glow;
  }
  return vec3f(0.32, 0.31, 0.3) * light;
}

// (arch_on, whether a block has the shopping street's arch, is in state.wgsl.)

// The ray against the arch of row n: t and the part (0 a post, 1 the board, 2 its cap).
fn arch_hit(o: vec3f, inv: vec3f, n: f32) -> vec2f {
  let z0 = n * UNIT + 0.4;
  var best = vec2f(FAR, 0.0);
  for (var k = 0; k < 4; k++) {
    var lo = vec3f(-HALF + 0.23, 0.0, z0);
    var hi = vec3f(-HALF + 0.47, 5.6, z0 + 0.3);
    var part = 0.0;
    if (k == 1) {
      lo = vec3f(HALF - 0.47, 0.0, z0);
      hi = vec3f(HALF - 0.23, 5.6, z0 + 0.3);
    } else if (k == 2) {
      lo = vec3f(-HALF + 0.15, 5.6, z0);
      hi = vec3f(HALF - 0.15, 7.0, z0 + 0.3);
      part = 1.0;
    } else if (k == 3) {
      lo = vec3f(-HALF - 0.1, 7.0, z0 - 0.1);
      hi = vec3f(HALF + 0.1, 7.15, z0 + 0.4);
      part = 2.0;
    }
    let r = box_hit(o, inv, lo, hi);
    if (r.x > 0.0 && r.x < best.x) {
      best = vec2f(r.x, part);
    }
  }
  return best;
}

fn arch_face(n: f32, part: f32, p: vec3f, aa: f32) -> vec3f {
  let light = params.ambient * vec3f(0.9, 0.95, 1.1) * 2.0 + street_light(vec3f(p.x, 0.0, p.z), true) * 0.2;
  if (part < 0.5 || part > 1.5) {
    return vec3f(0.12, 0.12, 0.13) * light;
  }
  // The board: red, its name (one of four, then the shopping street's name) in white letters lit
  // from within, a row of bulbs along its top and its foot.
  let z0 = n * UNIT + 0.4;
  var c = vec3f(0.5, 0.03, 0.03) * (light + vec3f(0.08, 0.01, 0.01));
  let bulb_row = min(abs(p.y - 5.7), abs(p.y - 6.9));
  let bulb = smoothstep(0.05 + aa, 0.03, length(vec2f(fract(p.x / 0.3 + 0.5) - 0.5, bulb_row / 0.3) * 0.3));
  c += vec3f(1.0, 0.75, 0.4) * 3.0 * bulb;
  if (p.z < z0 + 0.02) {
    let name = 33.0 + floor(h21(vec2f(floor(n / BLOCK), 62.7)) * 4.0);
    let h = 0.8;
    let ln = word_length(name);
    let left = -0.5 * (ln + 3.0) * h;
    let v = 6.7 - p.y;
    let first = horizontal_word(FACE_NOTO, name, p.x - left, v, ln * h, h, aa, 0.0);
    let second = horizontal_word(FACE_NOTO, 32.0, p.x - left - ln * h, v, 3.0 * h, h, aa, 0.0);
    c = mix(c, vec3f(1.6, 1.5, 1.4), max(first, second));
  }
  return c;
}

// The torii over the street, as the roof torii is drawn (scene()): vermilion, the kasagi's top
// black, lit from below by the street's lights and a warm glow at its feet, and faintly by the
// city. `piece` 0 a pillar, 1 the nuki, 2 the kasagi; `face` 1 across, 2 along, 3 top.
fn street_torii_face(piece: f32, face: f32, p: vec3f) -> vec3f {
  var albedo = vec3f(0.55, 0.06, 0.025);
  if (piece > 1.5 && face > 2.5) {
    albedo = vec3f(0.02);
  }
  let dim = select(select(0.8, 1.0, face < 1.5), 0.6, face > 2.5);
  let light = params.ambient * vec3f(0.9, 0.95, 1.1) * 4.0 + street_light(vec3f(p.x, 0.0, p.z), true) * 0.3 * exp(-p.y * 0.25) + vec3f(0.45, 0.24, 0.12) * 0.6 * exp(-p.y * 0.5);
  return albedo * light * dim;
}

// A standing board before shop (n, s), facing the way the street is walked: an A-board, dark in a
// wooden frame, its word chalked down it in a marker hand and lines of a menu under it; or a lit
// stand (a lit stand sign), a box lit from within, its word or one or two big characters on both
// faces (lettered()), of the shop's own height. Where it stands (x, z of its middle), its half
// width, height and half depth; the kind is the depth (0.17 an A-board).
fn board_of(n: f32, s: f32) -> vec4f {
  let id = vec2f(n, s);
  let zc = n * UNIT + 1.2 + 2.0 * h21(id + 93.1);
  if (h21(id + 93.9) < 0.45) {
    return vec4f(s * (HALF - 0.55), zc, 0.24, 0.92);
  }
  return vec4f(s * (HALF - 0.5), zc, 0.19 + 0.05 * h21(id + 94.7), 1.0 + 0.5 * h21(id + 95.3));
}

fn board_depth(b: vec4f) -> f32 {
  return select(0.13, 0.17, b.z > 0.235);
}

// The ray against it: t.
fn board_hit(o: vec3f, inv: vec3f, n: f32, s: f32) -> f32 {
  let b = board_of(n, s);
  let hz = board_depth(b);
  let r = box_hit(o, inv, vec3f(b.x - b.z, 0.0, b.y - hz), vec3f(b.x + b.z, b.w, b.y + hz));
  return select(FAR, r.x, r.x > 0.0);
}

fn board_face(n: f32, s: f32, p: vec3f, aa: f32) -> vec3f {
  let id = vec2f(n, s);
  let light = shop_colour(n, s) * 0.6 + params.ambient * vec3f(0.9, 0.95, 1.1) * 2.0;
  let b = board_of(n, s);
  let hz = board_depth(b);
  let x = p.x - (b.x - b.z);
  let front = abs(abs(p.z - b.y) - hz) < 0.01;
  if (hz > 0.15) {
    if (p.z > b.y - hz + 0.01) {
      return vec3f(0.2, 0.13, 0.07) * light;
    }
    let frame_band = 1.0 - rv_band(x, 0.04, 0.44, aa) * rv_band(p.y, 0.1, b.w - 0.04, aa);
    var words = array<f32, 6>(9.0, 37.0, 38.0, 20.0, 6.0, 30.0);
    let w = words[u32(h21(id + 94.3) * 6.0)];
    let chalk = vertical_word(FACE_MAGIC, w, x - 0.08, b.w - 0.08 - p.y, 0.32, 0.5, aa, 0.0);
    let menu = rv_band(p.y, 0.16, 0.34, aa) * rv_band(fract((p.y - 0.16) / 0.06), 0.4, 0.6, aa / 0.06) * rv_band(x, 0.1, 0.38, aa);
    var c = mix(vec3f(0.03, 0.05, 0.04), vec3f(0.85, 0.85, 0.8), max(chalk, menu * 0.6) * 0.9);
    c = mix(c, vec3f(0.25, 0.15, 0.08), frame_band);
    return c * light;
  }
  // A lit stand: its two faces lettered, its sides the colour, dimmer, its top and foot dark.
  let glow = glow_now() * (0.8 + 0.3 * h21(id + 2.8));
  if (p.y > b.w - 0.01 || p.y < 0.1) {
    return vec3f(0.03) * light;
  }
  if (!front) {
    return neon(h21(id + 4.7)) * 0.35 * glow;
  }
  let word = select(word_for(id, 4.0), big_word(id, h21(id + 9.7) < 0.5), h21(id + 9.9) < 0.45);
  let col = lettered(id, word, face_of(id), true, x, b.w - p.y, 2.0 * b.z, b.w - 0.1, floor(h21(id + 3.6) * 4.0), select(1.0, 2.0, h21(id + 13.1) < 0.2), 0.0, aa) * glow;
  return mix(col * 0.03 + vec3f(0.01), col, lit(id));
}

// Bicycles parked in a row before shop (n, s), side by side, their backs to the wall (seen side
// on from the street: two wheels, a step-through frame, a basket in front). Bicycle k: how many,
// where it stands along the street, and its outline (negative inside), q along it from its back
// wheel and up.
fn bikes_of(n: f32, s: f32) -> f32 {
  return 2.0 + floor(h21(vec2f(n, s) + 95.1) * 3.0);
}

fn bike_z(n: f32, k: f32) -> f32 {
  return n * UNIT + 0.8 + k * 0.5;
}

fn bike(q: vec2f) -> f32 {
  let r = 0.3;
  let back = abs(length(q - vec2f(0.0, r)) - r) - 0.022;
  let front = abs(length(q - vec2f(1.0, r)) - r) - 0.022;
  var f = min(back, front);
  f = min(f, rv_sd_segment(q, vec2f(0.0, r), vec2f(0.45, 0.28)) - 0.018);
  f = min(f, rv_sd_segment(q, vec2f(0.45, 0.28), vec2f(0.86, 0.86)) - 0.02);
  f = min(f, rv_sd_segment(q, vec2f(0.86, 0.86), vec2f(1.0, r)) - 0.018);
  f = min(f, rv_sd_segment(q, vec2f(0.4, 0.3), vec2f(0.33, 0.88)) - 0.018);
  f = min(f, rv_sd_segment(q, vec2f(0.24, 0.9), vec2f(0.42, 0.9)) - 0.03);
  f = min(f, rv_sd_segment(q, vec2f(0.86, 0.86), vec2f(0.76, 1.04)) - 0.016);
  f = min(f, rv_sd_box(q - vec2f(1.04, 0.86), vec2f(0.13, 0.1)));
  return f;
}

fn bike_hit(o: vec3f, d: vec3f, n: f32, s: f32, k: f32, pix: f32) -> f32 {
  let t = (bike_z(n, k) - o.z) / d.z;
  if (t <= 0.0) {
    return FAR;
  }
  let p = o + d * t;
  let q = vec2f(s * (s * (HALF - 0.32) - p.x), p.y);
  if (q.x < -0.4 || q.x > 1.4 || q.y > 1.15) {
    return FAR;
  }
  if (bike(q) < 0.5 * t * length(d) * pix) {
    return t;
  }
  return FAR;
}

fn bike_face(n: f32, s: f32, k: f32, p: vec3f) -> vec3f {
  let light = street_light(vec3f(p.x, 0.0, p.z), true) * 0.3 + params.ambient * vec3f(0.9, 0.95, 1.1);
  let paint = select(vec3f(0.3, 0.3, 0.32), vec3f(0.35, 0.06, 0.05), h21(vec2f(n * 3.0 + k, s) + 7.7) < 0.3);
  return paint * light * 0.8;
}

// The things in the street along the ray from the camera (o is always the camera: the rows come
// from the tables update fills for it), nearer than `best`: t, kind and two numbers naming the
// thing. Small things far off are left out, dissolving in a dither pattern (`dither`, 0..1 for
// the pixel) over 15 m past params.detail (less at lower quality); people from 1.6 times as far.
fn hit_street(o: vec3f, d: vec3f, best: f32, steps: f32, pix: f32, dither: f32) -> vec4f {
  var r = vec4f(best, 0.0, 0.0, 0.0);
  if (d.z < 1e-3) {
    return r;
  }
  let inv = inverse(d);
  let len = length(d);
  let n0 = state.row0;
  // At lower quality (frame.quality below 1) small things and people dissolve nearer, down to
  // 0.6 of the distance at the lowest.
  let detail = params.detail * mix(0.6, 1.0, frame.quality);
  // Over the middle of the street (within 1.85 m of it) nothing stands higher than the arch
  // (7.15 m): while the ray runs there (from t = clear.x to clear.y, kept a little inside) the
  // rows are passed over at once.
  var clear = vec2f(-1e9, 1e9);
  if (abs(d.x) > 1e-6) {
    let ta = (-1.4 - o.x) / d.x;
    let tb = (1.4 - o.x) / d.x;
    clear = vec2f(min(ta, tb), max(ta, tb));
  } else if (abs(o.x) >= 1.4) {
    clear = vec2f(1.0, -1.0);
  }
  if (abs(d.y) > 1e-6) {
    let ty = (7.3 - o.y) / d.y;
    clear = select(vec2f(clear.x, min(clear.y, ty)), vec2f(max(clear.x, ty), clear.y), d.y > 0.0);
  } else if (o.y <= 7.3) {
    clear = vec2f(1.0, -1.0);
  }
  // Row 0 of the tables is the row behind the camera's, whose things the ray never meets.
  for (var i = 1u; i < ROWS; i++) {
    let n = n0 + f32(i);
    let ta = (n * UNIT - o.z) / d.z;
    if (f32(i) >= steps || ta > r.x) {
      break;
    }
    if (ta >= clear.x && ((n + 1.0) * UNIT - o.z) / d.z <= clear.y) {
      // On to the last row the ray runs through clear (a row short, against rounding).
      let last = floor((o.z + d.z * clear.y) / UNIT) - 2.0;
      i = max(i, u32(max(last - n0, 0.0)));
      continue;
    }
    // How far the row is, and whether its small things and its people are drawn.
    let far = max(ta, 0.0) * len - detail;
    let small = far < 15.0 * dither;
    let people = far < 0.6 * detail + 20.0 * dither;
    // Where the ray runs within the row (from where it enters it, or the camera, to where it
    // leaves it): each kind of thing stands in its own part of the row, and one the ray cannot
    // reach is passed by before any of its sums are done.
    let pa = o + d * max((n * UNIT - o.z) / d.z, 0.0);
    let pb = o + d * (((n + 1.0) * UNIT - o.z) / d.z);
    let lo = min(pa.xy, pb.xy);
    let hi = max(pa.xy, pb.xy);
    for (var k = 0u; k < 2u; k++) {
      let s = f32(k) * 2.0 - 1.0;
      let row = i * 2u + k;
      // This side's wall, what stands before it, an awning, a pole.
      let shop = state.row_shop[row];
      let wall = shop.x;
      // How far out towards this side's wall the ray gets in the row.
      let out = select(-lo.x, hi.x, s > 0.0);
      // Signs hang from the walls, from 3 m up.
      if (out > 1.4 && hi.y > 3.2) {
        let ts = hit_sign(o, d, s, state.row_sign[row], wall);
        if (ts < r.x) {
          r = vec4f(ts, 4.0, n, s);
        }
      }
      if (crossing(n)) {
        // The traffic light at a corner of the cross street, its arm over the road, under 5.85 m.
        if (s == signal_side(n) && lo.y < 5.85) {
          let tl = signal_hit(o, d, n * UNIT + 0.25, s, pix);
          if (tl.x < r.x) {
            r = vec4f(tl.x, 5.0 + 0.01 * tl.y, n, s);
          }
        }
      } else if (out > HALF - 1.0 && lo.y < 2.6 && hi.y > 2.3 && shop.z > 0.5) {
        // Awnings jut a metre from the walls, 2.3 to 2.6 m up.
        let x0 = select(-wall, wall - 1.0, s > 0.0);
        let ra = box_hit(o, inv, vec3f(x0, 2.3, n * UNIT + 0.15), vec3f(x0 + 1.0, 2.6, n * UNIT + UNIT - 0.15));
        if (ra.x > 0.0 && ra.x < r.x) {
          r = vec4f(ra.x, 5.0 + ra.y, n, s);
        }
      }
      // On the pavement before the shop, under 2.2 m: its banners, board, bicycles or shrine.
      if (small && out > HALF - 2.0 && lo.y < 2.2) {
        let what = shop.y;
        if (what == 1.0 && out > HALF - 0.85) {
          for (var j = 0.0; j < 2.0; j += 1.0) {
            if (j > 0.5 && h21(vec2f(n, s) + 96.1) < 0.5) {
              break;
            }
            let tb = banner_hit(o, d, n, s, j);
            if (tb.x < r.x) {
              r = vec4f(tb.x, 11.0 + 0.5 * tb.y, n + 0.5 * j + tb.z * 0.2, s);
            }
          }
        } else if (what == 2.0 && out > HALF - 0.85 && lo.y < 1.5) {
          let tb = board_hit(o, inv, n, s);
          if (tb < r.x) {
            r = vec4f(tb, 14.0, n, s);
          }
        } else if (what == 3.0 && lo.y < 1.2) {
          for (var b = 0.0; b < 4.0; b += 1.0) {
            if (b >= bikes_of(n, s)) {
              break;
            }
            let tb = bike_hit(o, d, n, s, b, pix);
            if (tb < r.x) {
              r = vec4f(tb, 15.0, n + 0.2 * b, s);
            }
          }
        } else if (what == 4.0 && lo.y < 1.6) {
          let tb = shrine_hit(o, d, inv, n, s);
          if (tb.x < r.x) {
            r = vec4f(tb.x, 12.0 + 0.1 * tb.y, n, s);
          }
        } else if (what == 5.0 && out > wall - 0.8 && lo.y < 1.9) {
          for (var v = 0.0; v < 3.0; v += 1.0) {
            if (v >= vendings_of(n, s)) {
              break;
            }
            let tv = vending_hit(o, inv, n, s, v);
            if (tv < r.x) {
              r = vec4f(tv, 17.0, n + 0.2 * v, s);
            }
          }
        }
      }
      // What stands out from the front above the shop: within a metre of the wall, above 3 m.
      if (small && !crossing(n) && hi.y > 3.1 && out > wall - 1.0) {
        let tf = front_parts(o, inv, n, s, state.row_front[row], wall, lo.y, hi.y);
        if (tf.x < r.x) {
          r = vec4f(tf.x, 16.0 + 0.1 * tf.y, n, s);
        }
      }
      // Poles stand at the kerb, under 9.5 m, their arms reaching 0.85 m.
      if (out > HALF - 1.2 && lo.y < 9.5 && shop.w > 0.5) {
        let tp = hit_pole(o, d, n, s);
        if (tp < r.x) {
          r = vec4f(tp, 9.0, n, s);
        }
      }
    }
    // A torii over the street where the row holds one, under 6.7 m: t, the face and the piece
    // (torii_hit), kind 18.
    if (lo.y < 6.7 && street_torii(n)) {
      let tt = torii_hit(o, inv, vec4f(0.0, (n + 0.5) * UNIT, STREET_TORII, 0.0));
      if (tt.x > 0.0 && tt.x < r.x) {
        r = vec4f(tt.x, 18.0 + 0.1 * tt.z, n, tt.y);
      }
    }
    // The arch over the street at the start of a block, under 7.2 m.
    if (lo.y < 7.2 && arch_on(n)) {
      let ta = arch_hit(o, inv, n);
      if (ta.x < r.x) {
        r = vec4f(ta.x, 13.0 + 0.1 * ta.y, n, 0.0);
      }
    }
    // The burst's subject, in its one row (hero_row): a bicycle, a traffic light or a street lamp
    // (vending machines and a shrine stand as the things before the shops do).
    if (hero_row(n)) {
      let th = hero_hit(o, d, n, pix);
      if (th.x < r.x) {
        r = vec4f(th.x, 19.0 + 0.01 * th.y, n, state.hero.z);
      }
    }
    // People walk the road, under 2 m.
    if (people && lo.y < 2.0 && lo.x < ROAD + 0.2 && hi.x > -ROAD - 0.2) {
      for (var k = 0u; k < 3u; k++) {
        for (var j = 0u; j < 2u; j++) {
          let who = state.row_people[i * 6u + k * 2u + j];
          if (who.z <= 0.0) {
            continue;
          }
          // person_in_row(n, k, j), without stream()'s table (an array indexed per pixel lives in
          // slow memory).
          let pace = select(select(0.95, -1.15, k == 1u), 1.25, k == 0u);
          let m = floor((n * UNIT - pace * frame.time) / UNIT) + f32(j);
          let tq = hit_person(o, d, who, m, f32(k), pix);
          if (tq < r.x) {
            r = vec4f(tq, 10.0, m, f32(k));
          }
        }
      }
    }
  }
  return r;
}

// Lantern strings before tmax: the colour they add and how much of the pixel they cover.
fn strings(o: vec3f, d: vec3f, tmax: f32, pix: f32) -> vec4f {
  if (d.z < 1e-3) {
    return vec4f(0.0);
  }
  let len = length(d);
  let m0 = floor((o.z - 2.1) / (2.0 * UNIT));
  for (var i = 0.0; i < 12.0; i += 1.0) {
    let m = m0 + i;
    let t = (string_z(m) - o.z) / d.z;
    if (t > tmax) {
      break;
    }
    if (t <= 0.0) {
      continue;
    }
    let p = o + d * t;
    let footprint = t * len * pix;
    // The strings hang between 4.65 and 6.1 m (see the miss below): a ray past them, rising
    // above or falling below faster than its footprint grows, meets none further on.
    let reach = 0.6 + 1.4 * footprint;
    let steep = abs(d.y) >= 1.4 * len * pix;
    if (steep && ((d.y > 0.0 && p.y - 6.1 >= reach) || (d.y < 0.0 && 4.65 - p.y >= reach))) {
      break;
    }
    if (!string_on(m) || abs(p.x) > HALF) {
      continue;
    }
    let wire_y = string_y(m, p.x);
    // A lantern hangs within 0.24 + 0.21 m (and its edge's blur) of where its cord meets the
    // string, which sags less than 0.11 m between there and here: a ray passing further from the
    // string meets neither.
    if (abs(p.y - wire_y) >= reach) {
      continue;
    }
    let wire = smoothstep(0.012 + footprint, 0.0, abs(p.y - wire_y)) * min(0.012 / (0.012 + footprint), 1.0) * 2.0;
    // Each lantern hangs from the string and swings on its cord with the beat (a little more or
    // less than its neighbours), turning about where it hangs.
    let k = round(p.x / 0.72);
    let a = state.sway * mix(0.7, 1.15, h21(vec2f(m, k) + 6.1));
    let top = vec2f(k * 0.72, string_y(m, k * 0.72));
    let c = top + vec2f(sin(a), -cos(a)) * 0.24;
    let q = vec2f(cos(a) * (p.x - c.x) + sin(a) * (p.y - c.y), -sin(a) * (p.x - c.x) + cos(a) * (p.y - c.y)) / vec2f(0.15, 0.21);
    let r = length(q);
    let aa = footprint / 0.15;
    let lantern = smoothstep(1.0 + aa, 1.0 - aa, r) * step(abs(k * 0.72), HALF - 0.4);
    if (lantern > 0.0 || wire > 0.0) {
      let colour = lantern_colour(m) * (1.3 - 0.6 * r * r) * (0.75 + 0.25 * cos(q.y * 9.0));
      return vec4f(colour * lantern, max(lantern, min(wire, 1.0) * 0.9));
    }
  }
  return vec4f(0.0);
}

// ---- Sky and air ----

fn sky(d: vec3f) -> vec3f {
  let up = d.y;
  var c = mix(vec3f(0.06, 0.047, 0.038), vec3f(0.007, 0.009, 0.016), smoothstep(-0.02, 0.5, up));
  // Low clouds, lit from below by the city; none below the horizon or high up.
  if (up < 0.0 || up >= 0.6) {
    return c;
  }
  let q = d.xz / max(up, 0.05);
  // Their finest detail goes first as the quality drops.
  let octaves = select(select(4, 3, frame.quality < 0.75), 2, frame.quality < 0.25);
  let cloud = smoothstep(-0.1, 0.5, rv_fbm(q * 0.25 + vec2f(frame.time * 0.01, 0.0), octaves));
  c += vec3f(0.05, 0.04, 0.032) * cloud * smoothstep(0.6, 0.05, up) * step(0.0, up);
  return c;
}

fn fogged(c: vec3f, dist: f32, d: vec3f) -> vec3f {
  let haze = mix(vec3f(0.055, 0.045, 0.04), vec3f(0.02, 0.022, 0.03), smoothstep(0.0, 0.4, d.y));
  return mix(c, haze, 1.0 - exp(-dist * 0.012 * params.fog));
}

// ---- The street's surface ----

// A shopping street's paving, as Japanese shotengai are paved: interlocking blocks in a running
// bond, a band of light blocks with a diamond laid in blocks every two rows (it marks the
// distance as the camera glides), a yellow guide line for the blind with warning dots before each
// cross street, steel gutters along the shops, and now and then a manhole. The albedo, and an id
// per block.
fn paving(p: vec3f, n: f32, aa: f32) -> vec4f {
  let ax = abs(p.x);
  let far = smoothstep(0.01, 0.06, aa);
  // Steel gutters along the shops: slots across, and their frame.
  if (ax > ROAD && ax < ROAD + 0.24) {
    let slot = rv_band(fract(p.z / 0.035), 0.25, 0.75, aa / 0.035);
    let frame = 1.0 - rv_band(ax, ROAD + 0.025, ROAD + 0.215, aa);
    let steel = mix(vec3f(0.11, 0.105, 0.1), vec3f(0.012), slot * (1.0 - far) * 0.85);
    return vec4f(mix(steel, vec3f(0.2, 0.19, 0.18), frame), 0.0);
  }
  // The guide line, on one side of the street: bars along it, dots before a cross street.
  if (p.x > 1.55 && p.x < 1.85) {
    let tile = smoothstep(0.004, 0.01 + aa, min(abs(fract(p.z / 0.3) - 0.5), 0.5 - abs(p.x - 1.7) / 0.3) * 0.3);
    let warn = crossing(n + 1.0) && p.z > (n + 1.0) * UNIT - 0.9;
    var bump: f32;
    if (warn) {
      let q = fract(vec2f(p.x - 1.55, p.z) / 0.06) - 0.5;
      bump = smoothstep(0.32, 0.22, length(q));
    } else {
      bump = rv_band(fract((p.x - 1.55) / 0.075), 0.3, 0.7, aa / 0.075);
    }
    let yellow = vec3f(0.62, 0.45, 0.04) * (0.85 + 0.25 * bump * (1.0 - far));
    return vec4f(mix(vec3f(0.08, 0.06, 0.02), yellow, mix(tile, 1.0, far)), 0.0);
  }
  // A manhole in some rows.
  if (h21(vec2f(n, 3.3)) < 0.4) {
    let c = vec2f((h21(vec2f(n, 4.4)) - 0.5) * 2.4, n * UNIT + 1.0 + 2.5 * h21(vec2f(n, 5.5)));
    let r = length(p.xz - c);
    if (r < 0.33) {
      let q = (p.xz - c) / 0.05;
      let grip = step(0.5, fract(q.x + 0.5 * floor(q.y))) * rv_band(r, 0.0, 0.25, aa);
      let ring = rv_band(r, 0.26, 0.3, aa);
      let iron = vec3f(0.1, 0.095, 0.09) * (1.0 + 0.6 * grip * (1.0 - far)) + vec3f(0.08) * ring;
      return vec4f(mix(vec3f(0.03), iron, smoothstep(0.33, 0.32 - aa, r)), 0.0);
    }
  }
  // The blocks, 0.2 m along and 0.1 m across; the pattern is read at each block's centre so it
  // is laid in whole blocks.
  let row = floor(p.x / 0.1);
  let shift = 0.5 * step(0.25, fract(row * 0.5));
  let zz = p.z / 0.2 + shift;
  let bi = floor(zz);
  let f = vec2f(fract(zz) * 0.2, (p.x / 0.1 - row) * 0.1);
  let joint = smoothstep(0.003, 0.009 + aa, min(min(f.x, 0.2 - f.x), min(f.y, 0.1 - f.y)));
  let c = vec2f((row + 0.5) * 0.1, (bi + 0.5 - shift) * 0.2);
  let hv = h21(vec2f(bi, row));
  // Close in tone, so that posterizing never breaks the street into noise.
  var block = vec3f(0.24, 0.215, 0.19) * (0.94 + 0.1 * hv);
  if (ax > ROAD) {
    // In front of the shops: smaller, darker square tiles.
    let t = abs(fract(p.xz / 0.3) - 0.5) * 0.3;
    let m = smoothstep(0.0, 0.01 + aa, 0.15 - max(t.x, t.y));
    let tile = vec3f(0.15, 0.145, 0.14) * (0.95 + 0.1 * h21(floor(p.xz / 0.3)));
    return vec4f(mix(mix(vec3f(0.05), tile, m), tile * 0.9, far), 0.0);
  }
  let zb = c.y - 2.0 * UNIT * floor(c.y / (2.0 * UNIT));
  if (zb < 1.4) {
    let d = abs(c.x) / 1.3 + abs(zb - 0.7) / 0.7;
    block = vec3f(0.4, 0.37, 0.32) * (0.96 + 0.06 * hv);
    if (d < 1.0 && d > 0.55) {
      block = vec3f(0.3, 0.14, 0.1);
    } else if (d <= 0.3) {
      block = vec3f(0.46, 0.34, 0.13);
    }
  }
  let col = mix(vec3f(0.05, 0.045, 0.04), block, joint);
  return vec4f(mix(col, block * 0.88, far), hv + row * 0.37);
}

// The sign and the wall of shop (n, s), from the tables update fills (State's row_sign, row_shop)
// where they reach.
fn sign_at(n: f32, s: f32) -> vec4f {
  let i = n - state.row0;
  if (i >= 0.0 && i < f32(ROWS)) {
    return state.row_sign[u32(i) * 2u + u32(s > 0.0)];
  }
  return sign_of(n, s);
}

fn wall_at(n: f32, s: f32) -> f32 {
  let i = n - state.row0;
  if (i >= 0.0 && i < f32(ROWS)) {
    return state.row_shop[u32(i) * 2u + u32(s > 0.0)].x;
  }
  return wall_x(n, s);
}

// What lights things near the ground at p: shops (unless `shops` is false), signs and lantern
// strings nearby.
fn street_light(p: vec3f, shops: bool) -> vec3f {
  var e = params.ambient * vec3f(0.9, 0.95, 1.1);
  let n0 = floor(p.z / UNIT);
  // Two rows either way; at half quality and below only one.
  let rows = select(2.0, 1.0, frame.quality < 0.5);
  for (var i = -rows; i <= rows; i += 1.0) {
    let n = n0 + i;
    for (var k = 0.0; k < 2.0; k += 1.0) {
      let s = k * 2.0 - 1.0;
      let wall = wall_at(n, s);
      let sp = vec3f(s * (wall - 0.1), 1.2, n * UNIT + 0.5 * UNIT);
      let ds = sp - p;
      e += select(vec3f(0.0), shop_light(n, s), shops) * 1.1 * sp.y / pow(dot(ds, ds) + 1.0, 1.5);
      let sg = sign_at(n, s);
      if (sg.y > 0.0) {
        let lp = vec3f(s * (wall - 0.12 - 0.5 * sg.y), 0.5 * (sg.z + sg.w), sg.x);
        let dl = lp - p;
        e += sign_light(n, s) * (sg.w - sg.z) * 0.6 * lp.y / pow(dot(dl, dl) + 0.5, 1.5);
      }
    }
  }
  return e + lantern_light(p) + hero_light(p);
}

// The lantern strings' light at p, as lines of light across the street.
fn lantern_light(p: vec3f) -> vec3f {
  var e = vec3f(0.0);
  let m0 = floor((p.z - 2.1) / (2.0 * UNIT));
  for (var m = m0; m <= m0 + 1.0; m += 1.0) {
    if (string_on(m) && abs(p.x) < HALF) {
      let h = string_y(m, p.x) - 0.24;
      let dz = string_z(m) - p.z;
      e += lantern_colour(m) * 0.35 * h / (dz * dz + h * h);
    }
  }
  return e;
}

// The light a shop throws out of its open front onto the street, as background painters lay it
// in: a pool the width of the shop, its edges softening and its light fading away from the door.
// A shutter throws none, so the street is bright and dark by turns.
fn shop_spill(p: vec3f) -> vec3f {
  var e = vec3f(0.0);
  let n0 = floor(p.z / UNIT);
  for (var i = -1.0; i <= 1.0; i += 1.0) {
    let n = n0 + i;
    for (var k = 0.0; k < 2.0; k += 1.0) {
      let s = k * 2.0 - 1.0;
      let dx = wall_x(n, s) - s * p.x;
      if (dx < 0.0) {
        continue;
      }
      let out = max(max(n * UNIT + 0.3 - p.z, p.z - (n * UNIT + UNIT - 0.3)), 0.0);
      let side = 1.0 - smoothstep(0.0, 0.2 + 0.6 * dx, out);
      e += shop_colour(n, s) * side / (1.0 + 0.3 * dx * dx);
    }
  }
  return e;
}

// What lights the ground at p: only the shops' pools and a little of the lantern strings, all
// steady. The signs high up light it too faintly to show, and nothing on it beats: the street is
// the calm ground the lights above dance over.
fn ground_light(p: vec3f) -> vec3f {
  return params.ambient * vec3f(0.9, 0.95, 1.1) + shop_spill(p) * params.spill + lantern_light(p) * 0.5 + hero_light(p);
}

// The wet street mirrors the lights, blurred: a sign, a shop window or a lantern string is a
// glowing rectangle, and a point of the street gets its light by how far its mirrored ray misses
// the rectangle, measured as an angle, little across and much up and down. So every sign leaves
// a soft streak of its own colour running towards the camera, as on a wet street at night, and
// nothing else on the street is mirrored (no words, no shapes). Light reflected off a wet street
// is as bright near and far: it does not fall off with distance.

struct Blur {
  // The mirrored ray, the spread across and up and down (radians).
  o: vec3f,
  r: vec3f,
  across: f32,
  up: f32,
}

// How much of a rectangle at z = plane (x from x0 to x1, y from y0 to y1) the blurred ray sees.
fn facing_z(b: Blur, plane: f32, x0: f32, x1: f32, y0: f32, y1: f32) -> f32 {
  let t = (plane - b.o.z) / b.r.z;
  if (t <= 0.05) {
    return 0.0;
  }
  let h = b.o + b.r * t;
  let miss = vec2f(h.x - clamp(h.x, min(x0, x1), max(x0, x1)), h.y - clamp(h.y, y0, y1)) / t;
  // A rectangle narrower than the spread lights less of it.
  let cover = min(abs(x1 - x0) / t / (1.8 * b.across), 1.0);
  return exp(-dot(miss / vec2f(b.across, b.up), miss / vec2f(b.across, b.up))) * cover;
}

// The same for a rectangle on a wall at x = plane (z from z0 to z1, y from y0 to y1).
fn facing_x(b: Blur, plane: f32, z0: f32, z1: f32, y0: f32, y1: f32) -> f32 {
  let t = (plane - b.o.x) / b.r.x;
  if (t <= 0.05) {
    return 0.0;
  }
  let h = b.o + b.r * t;
  let miss = vec2f(h.z - clamp(h.z, z0, z1), h.y - clamp(h.y, y0, y1)) / t;
  return exp(-dot(miss / vec2f(b.across, b.up), miss / vec2f(b.across, b.up)));
}

// What the wet street at p mirrors, seen from the direction v, with the surface normal n and
// roughness rough (0 a mirror, 1 matte).
fn street_sheen(p: vec3f, v: vec3f, n: vec3f, rough: f32) -> vec3f {
  var b: Blur;
  b.o = p;
  b.r = reflect(-v, n);
  b.across = 0.006 + 0.09 * rough;
  b.up = 0.02 + 0.7 * rough;
  if (b.r.y <= 0.0) {
    return vec3f(0.0);
  }
  var e = vec3f(0.0);
  // The rows the ray passes on its way up past the signs, fading at both ends so no seam shows.
  let z0 = p.z - UNIT;
  let z1 = p.z + b.r.z / max(b.r.y, 0.04) * 14.0 + UNIT;
  let first = floor(min(z0, z1) / UNIT);
  let last = min(floor(max(z0, z1) / UNIT), first + 11.0);
  for (var nn = first; nn <= last; nn += 1.0) {
    let mid = nn * UNIT + 0.5 * UNIT;
    let fade = clamp((mid - min(z0, z1)) / UNIT + 0.5, 0.0, 1.0) * clamp((max(z0, z1) - mid) / UNIT + 0.5, 0.0, 1.0);
    var row = vec3f(0.0);
    for (var k = 0.0; k < 2.0; k += 1.0) {
      let s = k * 2.0 - 1.0;
      let w = wall_x(nn, s);
      let window = facing_x(b, s * w, nn * UNIT + 0.3, nn * UNIT + UNIT - 0.3, 0.2, 2.4);
      row += shop_colour(nn, s) * 0.8 * window;
      let sg = sign_of(nn, s);
      if (sg.y > 0.0) {
        let x0 = s * (w - 0.12);
        let seen = facing_z(b, sg.x, x0, x0 - s * sg.y, sg.z, sg.w);
        row += sign_colour(nn, s) * 0.8 * seen;
      }
    }
    e += row * fade;
  }
  let m0 = floor((min(z0, z1) - 2.1) / (2.0 * UNIT));
  for (var m = m0; m <= m0 + 6.0; m += 1.0) {
    if (string_on(m) && string_z(m) < max(z0, z1) + UNIT) {
      let y = string_y(m, 0.0) - 0.24;
      e += lantern_colour(m) * 0.45 * facing_z(b, string_z(m), -HALF, HALF, y - 0.25, y + 0.25);
    }
  }
  // Water mirrors more at a grazing angle.
  let fresnel = 0.02 + 0.98 * pow(1.0 - max(dot(n, v), 0.0), 5.0);
  return e * fresnel;
}

// The ground at p (the street, a cross street or an empty lot): its albedo, then the tilt of
// its surface and how wet it is.
fn ground(p: vec3f, aa: f32) -> mat2x3f {
  let ax = abs(p.x);
  let n = floor(p.z / UNIT);
  var albedo: vec3f;
  var tilt = vec2f(0.0);
  if (!crossing(n) && ax < wall_x(n, select(-1.0, 1.0, p.x > 0.0))) {
    let b = paving(p, n, aa);
    albedo = b.rgb;
    if (hero_row(n) && state.hero.x == HERO_SIGNAL && ax < ROAD) {
      // The crossing painted under the burst's traffic light.
      albedo = mix(albedo, vec3f(0.55), rv_band(fract(p.x / 0.9), 0.0, 0.5, aa / 0.9) * rv_band(p.z - n * UNIT, 1.7, 4.1, aa));
    }
    // Each block sits at its own small angle, so what the wet street mirrors breaks into blocks.
    tilt = (vec2f(h21(vec2f(b.w, 1.3)), h21(vec2f(b.w, 7.1))) - 0.5) * 0.06 * (1.0 - smoothstep(0.01, 0.05, aa)) * step(0.001, b.w);
  } else {
    // Asphalt; across the main street a zebra crossing, along the cross street a centre line.
    albedo = vec3f(0.045, 0.045, 0.05);
    let zl = p.z - n * UNIT;
    if (ax < HALF) {
      let stripe = rv_band(fract(p.x / 0.9), 0.0, 0.5, aa / 0.9) * rv_band(zl, 0.7, 3.8, aa);
      albedo = mix(albedo, vec3f(0.55), stripe);
    } else {
      albedo = mix(albedo, vec3f(0.5, 0.42, 0.1), rv_band(zl, 2.2, 2.3, aa) * step(0.5, fract(p.x / 3.0)));
    }
  }
  // Puddles here and there: dark, wet and smooth, rippling a little. They alone mirror much.
  // None at all when puddles is 0.
  var puddle = 0.0;
  if (params.puddles > 0.0) {
    puddle = smoothstep(0.08, 0.2, rv_fbm(p.xz * 0.3 + vec2f(3.1, 7.7), 4) + params.puddles * 0.25 - 0.2);
    tilt = mix(tilt, vec2f(rv_noise(p.xz * 6.0 + frame.time * 0.7), rv_noise(p.zx * 6.0 - frame.time * 0.6)) * 0.008, puddle);
  }
  albedo *= 1.0 - 0.6 * puddle;
  let wet = mix(0.35 * params.wet, 0.8, puddle);
  return mat2x3f(albedo, vec3f(tilt, wet));
}

// ---- Putting it together ----

// What a ray meets first, stopping at tmax (the ground), for scene() in the next pass (hit.wgsl
// finds it, street.wgsl draws it): as hit_street names a thing in the street (t, its kind 4..17
// and two numbers), or a building's t, 100 + 10 × its part + the face (trace's Hit), column and
// row; or tmax and 0 for nothing. `dither` (0..1, a pattern over the pixels) dissolves far small
// things (see hit_street).
fn find(o: vec3f, d: vec3f, pix: f32, grid_steps: f32, street_steps: f32, tmax: f32, dither: f32) -> vec4f {
  let hit = trace(o, d, grid_steps, tmax);
  let things = hit_street(o, d, hit.t, street_steps, pix, dither);
  if (things.y > 0.5) {
    return things;
  }
  if (hit.kind > 0.5) {
    return vec4f(hit.t, 100.0 + 10.0 * hit.part + hit.kind, f32(hit.i), hit.n);
  }
  return vec4f(hit.t, 0.0, 0.0, 0.0);
}

// The colour along a ray, stopping at tmax (the ground), and how far it went (FAR: the sky), for
// what it meets first (as find()).
fn scene(o: vec3f, d: vec3f, pix: f32, tmax: f32, met: vec4f) -> vec4f {
  let len = length(d);
  let dn = d / len;
  var hit: Hit;
  hit.t = met.x;
  hit.kind = 0.0;
  hit.part = 0.0;
  var things = met;
  if (met.y > 99.5) {
    let code = met.y - 100.0;
    hit.part = floor(code * 0.1);
    hit.kind = code - 10.0 * hit.part;
    hit.i = i32(met.z);
    hit.n = met.w;
    things = vec4f(met.x, 0.0, 0.0, 0.0);
  }
  let found = things.y > 0.5 || hit.kind > 0.5;
  if (!found) {
    return vec4f(sky(dn), select(FAR, tmax, tmax < FAR));
  }
  let t = things.x;
  let p = o + d * t;
  let dist = t * len;
  var col: vec3f;
  if (things.y > 18.5) {
    col = hero_face(things.z, things.w, round((things.y - 19.0) * 100.0), p, dist * pix);
  } else if (things.y > 17.5) {
    col = street_torii_face(round((things.y - 18.0) * 10.0), things.w, p);
  } else if (things.y > 16.5) {
    col = vending_face(floor(things.z), things.w, round(fract(things.z) * 5.0), p, dist * pix / max(abs(dn.x), 0.15));
  } else if (things.y > 15.5) {
    col = front_part_face(things.z, things.w, (things.y - 16.0) * 10.0, p, dn, dist * pix / max(abs(dn.x), 0.15));
  } else if (things.y > 14.5) {
    col = bike_face(floor(things.z), things.w, round(fract(things.z) * 5.0), p);
  } else if (things.y > 13.5) {
    col = board_face(things.z, things.w, p, dist * pix / max(abs(dn.z), 0.2));
  } else if (things.y > 12.5) {
    col = arch_face(things.z, (things.y - 13.0) * 10.0, p, dist * pix / max(abs(dn.z), 0.2));
  } else if (things.y > 11.5) {
    col = shrine_face(things.z, things.w, (things.y - 12.0) * 10.0, p);
  } else if (things.y > 10.5) {
    let j = floor(fract(things.z) * 2.0 + 1e-3);
    let a = (fract(things.z) - 0.5 * j) * 5.0;
    col = banner_face(floor(things.z), things.w, j, a, p, dist * pix / max(abs(dn.z), 0.2), things.y > 11.25);
  } else if (things.y > 9.5) {
    col = person_face(things.z, things.w, p, dn, pix);
  } else if (things.y > 8.5) {
    col = pole_face(p, things.w);
  } else if (things.y > 5.5) {
    col = awning_face(things.z, things.w, p, things.y - 5.0, dn);
  } else if (things.y > 4.5) {
    col = signal_face(round((things.y - 5.0) * 100.0), things.w, things.z * UNIT + 0.25, p, dist * pix);
  } else if (things.y > 3.5) {
    col = sign_face(things.z, things.w, p, dist * pix / max(abs(dn.z), 0.2));
  } else if (hit.part > 1.5) {
    // A roof torii: vermilion, the kasagi's top black, lit from below by a small light at its
    // feet and faintly by the city.
    let r = torii_of(hit.i, hit.n);
    let up = (p.y - r.w) / r.z;
    var albedo = vec3f(0.55, 0.06, 0.025);
    if (hit.part > 3.5 && hit.kind > 2.5) {
      albedo = vec3f(0.02);
    }
    let face = select(select(0.8, 1.0, hit.kind < 1.5), 0.6, hit.kind > 2.5);
    let light = params.ambient * vec3f(0.9, 0.95, 1.1) * 4.0 + vec3f(0.45, 0.24, 0.12) * exp(-up * 0.9);
    col = albedo * light * face;
  } else if (hit.part > 0.5) {
    // A billboard: its face to the street is a screen, the rest a dark frame.
    let s = f32(hit.i);
    col = vec3f(0.015, 0.015, 0.018);
    if (hit.kind < 1.5 && (s > 0.0) == (d.x > 0.0)) {
      let bb = billboard_of(hit.n, s);
      let z0 = first_row(hit.n, s) * UNIT + 0.4;
      let w = rows_of(hit.n, s) * UNIT - 0.8;
      let along = select(p.z - z0, w - (p.z - z0), s > 0.0);
      col = big_screen(vec2f(first_row(hit.n, s), s) + 0.71, along, bb.y - p.y, w, bb.y - bb.x, dist * pix / max(abs(dn.x), 0.08));
    }
  } else if (hit.kind > 2.5) {
    let span = column_span(hit.i);
    let xz = vec2f(p.x - span.x, p.z - hit.n * UNIT);
    col = roof(hit.i, hit.n, xz, vec2f(COL, UNIT), dist * pix / max(abs(dn.y), 0.1));
  } else {
    let faces_street = hit.kind < 1.5 && abs(hit.i) == 1 && (hit.i > 0) == (d.x > 0.0);
    let grazing = select(abs(dn.z), abs(dn.x), hit.kind < 1.5);
    let aa = dist * pix / max(grazing, 0.08);
    if (faces_street) {
      col = facade(hit.n, f32(hit.i), p.z - hit.n * UNIT, p.y, aa);
    } else {
      col = city_wall(hit.i, hit.n, p, hit.kind, aa);
    }
  }
  return vec4f(fogged(col, dist, dn), t);
}

// The camera's ray through a pixel: where it starts, its direction (not normalised) and the
// size of a pixel one unit away.
struct Ray {
  o: vec3f,
  d: vec3f,
  pix: f32,
}

fn camera_ray(px: Pixel) -> Ray {
  let p2 = rv_rotate(px.p, state.roll) * state.lens * 0.72;
  var d = vec3f(p2.x, p2.y, 1.0);
  // Pitch up for a positive pitch, yaw left for a positive yaw.
  let dyz = rv_rotate(d.yz, -state.pitch);
  d = vec3f(d.x, dyz.x, dyz.y);
  let dxz = rv_rotate(d.xz, state.yaw);
  d = vec3f(dxz.x, d.y, dxz.y);
  var ray: Ray;
  ray.o = state.pos;
  ray.d = d;
  ray.pix = state.lens * 0.72 / px.size.y * 1.2;
  return ray;
}
