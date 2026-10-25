// The tower, shared by the passes: plates stacked on a grid, thin and wide or squat, crowded at
// the core and straggling at the edge, tapering to a tip that grows. A ray walks the grid cell by
// cell (Amanatides and Woo) and meets at most one plate in each, exactly: nothing is marched.


// A cell's piece of a plate: the piece's box; the whole plate's box, for a plate several cells
// across, which its edges are rounded by and which is lit and textured as one; the plate's own
// number; whether it is there, its landing flash and its shade.
struct Plate {
  centre: vec3f,
  half: vec3f,
  whole_c: vec3f,
  whole_h: vec3f,
  key: vec3f,
  on: f32,
  fresh: f32,
  shade: f32,
}

// The tower's girth at height y: it swells and narrows on the way up, fixed for every height, so
// a plate once grown never changes.
fn girth_at(y: f32) -> f32 {
  // A slow swell up the tower, and each layer its own width on top: some narrow as a column,
  // some flung wide.
  let slow = rv_noise(vec2f(y * 0.02, 1.3));
  let layer = hash3(vec3f(0.0, floor(y / CELL.y), 41.0));
  let own = mix(0.25, 1.6, pow(layer, 1.3));
  return params.girth * (1.0 + 0.4 * slow) * mix(1.0, own, params.vary) + 1.0;
}

// When the plate of a cell appears: once the tip has grown past this height. Further from the
// axis a plate appears later, so the growing front is a cone and the tower comes to a point.
fn appears_at(mid: vec3f) -> f32 {
  let ring = max(abs(mid.x), abs(mid.z));
  return mid.y + ring * params.taper / params.girth;
}

// The square plates of layer y (its index): one to four cells across, strewn within the layer's
// girth, so they overlap from layer to layer like scales. Each is its corner cell, its size in
// cells, and whether it is there. Worked out once per layer as a ray crosses it.
struct Squares {
  a: vec4f,
  b: vec4f,
  c: vec4f,
  d: vec4f,
  e: vec4f,
}

fn square(y: f32, k: f32, r_max: f32) -> vec4f {
  let id = vec3f(0.0, y, k);
  let n = 1.0 + floor(pow(hash3(id + 1.5), 1.4) * 4.0);
  let room = max(r_max / CELL.x - n * 0.5, 0.0);
  let corner = floor((vec2f(hash3(id + 3.5), hash3(id + 4.5)) * 2.0 - 1.0) * room - n * 0.5 + 0.5);
  return vec4f(corner, n, f32(hash3(id + 0.5) <= params.density));
}

fn layer_plates(y: f32) -> Squares {
  let r_max = girth_at((y + 0.5) * CELL.y);
  return Squares(square(y, 0.0, r_max), square(y, 1.0, r_max), square(y, 2.0, r_max), square(y, 3.0, r_max), square(y, 4.0, r_max));
}

fn square_of(squares: Squares, slab: f32) -> vec4f {
  return select(select(select(select(squares.e, squares.d, slab < 3.5), squares.c, slab < 2.5), squares.b, slab < 1.5), squares.a, slab < 0.5);
}

fn in_square(c: vec2f, sq: vec4f) -> bool {
  let rel = c - sq.xy;
  return sq.w > 0.5 && all(rel >= vec2f(0.0)) && all(rel < vec2f(sq.z));
}

// One of three sizes for h in 0..1, the smallest most often: a few sizes read as made, sizes all
// different read as noise.
fn tier(h: f32, sizes: vec3f) -> f32 {
  return select(select(sizes.x, sizes.y, h > 0.5), sizes.z, h > 0.82);
}

// The plate a cell holds, where it rests, if any: one of its layer's square plates, or a small
// single plate scattered in between, all of many thicknesses.
fn plate_at_rest(c: vec3f, squares: Squares) -> Plate {
  var p: Plate;
  p.on = 0.0;
  p.fresh = 0.0;
  let mid = (c + 0.5) * CELL;
  let r_max = girth_at(mid.y);
  let ring = max(abs(mid.x), abs(mid.z));
  if (ring > r_max * 1.4 + 2.0 || (ring < params.core && r_max > params.core * 3.0)) {
    return p;
  }
  var slab = -1.0;
  if (in_square(c.xz, squares.a)) {
    slab = 0.0;
  } else if (in_square(c.xz, squares.b)) {
    slab = 1.0;
  } else if (in_square(c.xz, squares.c)) {
    slab = 2.0;
  } else if (in_square(c.xz, squares.d)) {
    slab = 3.0;
  } else if (in_square(c.xz, squares.e)) {
    slab = 4.0;
  }
  var half: vec3f;
  var centre = mid;
  var key = vec3f(0.0, c.y, slab);
  var whole_c = mid;
  var whole_h = vec3f(0.0);
  if (slab >= 0.0) {
    half = vec3f(CELL.x * 0.5, CELL.y * 0.5 * tier(hash3(key + 5.5), vec3f(0.1, 0.2, 0.38)), CELL.z * 0.5);
    // The whole square, its cells one plate.
    let sq = square_of(squares, slab);
    whole_c = vec3f((sq.x + sq.z * 0.5) * CELL.x, mid.y, (sq.y + sq.z * 0.5) * CELL.z);
    whole_h = vec3f(sq.z * CELL.x * 0.5, half.y, sq.z * CELL.z * 0.5);
  } else {
    // A small plate on its own: thick in the tower, loose and scarce outside it.
    let h = hash3(c);
    let inside = ring < r_max && h < params.scatter;
    let loose = ring >= r_max && h > 1.0 - params.loose * 0.04;
    if (!(inside || loose)) {
      return p;
    }
    key = c;
    let s = tier(hash3(c + 2.1), vec3f(0.4, 0.65, 0.9));
    half = vec3f(CELL.x * 0.5 * s, CELL.y * 0.5 * tier(hash3(c + 4.4), vec3f(0.1, 0.2, 0.38)), CELL.z * 0.5 * s);
    centre += vec3f(hash3(c + 5.5) - 0.5, 0.0, hash3(c + 7.7) - 0.5) * (CELL - 2.0 * half);
    whole_c = centre;
    whole_h = half;
  }
  let slack = CELL.y * 0.5 - half.y;
  // Each plate at its own height in the layer, so the layers do not read as slices.
  let lift = (hash3(key + 6.6) * 2.0 - 1.0) * slack * 0.95;
  centre.y += lift;
  whole_c.y = centre.y;
  p.centre = centre;
  p.half = max(half, vec3f(0.001));
  p.whole_c = whole_c;
  p.whole_h = max(whole_h, vec3f(0.001));
  p.key = key;
  p.on = 1.0;
  // Most plates are pale; some are dark, a few warm.
  let tone = hash3(key + 8.8);
  p.shade = select(select(select(0.0, 0.5, tone > 0.65), 1.0, tone > 0.95), -1.0, tone < 0.3);
  return p;
}

// The plate of cell c now: it appears on the beat the tip grows past it, and slams in from
// outside to its place in a fraction of a second (by up to a cell, so near the growing front the
// walk need only look one cell further in).
fn plate(c: vec3f, squares: Squares) -> Plate {
  let mid = (c + 0.5) * CELL;
  let due = appears_at(mid);
  var p: Plate;
  p.on = 0.0;
  p.fresh = 0.0;
  if (state.top_to < due) {
    return p;
  }
  p = plate_at_rest(c, squares);
  if (p.on < 0.5) {
    return p;
  }
  // Plates of the last beat's growth fly in: off the mark at once, hard into place (the outer
  // ones a touch later, so they land as a ripple). Older plates are at rest.
  var settle = 1.0;
  if (due > state.top_prev) {
    // (A plate several cells across lands as one.)
    let delay = 0.06 * hash3(p.key + 13.1);
    settle = clamp((state.since_beat - delay) / max(params.arrive, 0.02), 0.0, 1.0);
  }
  let away = pow(1.0 - settle, 4.0);
  let out = way_out(c);
  let shift = vec3f(out.x, 0.0, out.y) * away * CELL.x * 0.95 * params.fly;
  p.centre += shift;
  p.whole_c += shift;
  // A flash as it lands, fading.
  let landed = state.since_beat - params.arrive;
  p.fresh = f32(due > state.top_prev) * select(0.35 * (1.0 - settle), exp(-landed * 8.0), landed > 0.0);
  return p;
}

// The way out from the axis at cell c, along its larger side (the way its plate flies in from).
fn way_out(c: vec3f) -> vec2f {
  let mid = (c.xz + 0.5) * CELL.xz;
  if (max(abs(mid.x), abs(mid.y)) < 0.5) {
    return vec2f(1.0, 0.0);
  }
  return select(vec2f(sign(mid.x), 0.0), vec2f(0.0, sign(mid.y)), abs(mid.y) > abs(mid.x));
}

// Whether cell c is near the growing front, where plates may still be flying in.
fn arriving(c: vec3f) -> bool {
  let due = appears_at((c + 0.5) * CELL);
  return due > state.top_prev && due <= state.top_to && state.since_beat < params.arrive + 0.1;
}

// A box centred at c with half sizes s (Inigo Quilez's slab method): distance and normal.
fn box_hit(ro: vec3f, rd: vec3f, c: vec3f, s: vec3f) -> vec4f {
  let m = 1.0 / rd;
  let n = m * (ro - c);
  let k = abs(m) * s;
  let t1 = -n - k;
  let t2 = -n + k;
  let near = max(max(t1.x, t1.y), t1.z);
  let far = min(min(t2.x, t2.y), t2.z);
  if (near > far || far < 0.0) {
    return vec4f(1e9, 0.0, 1.0, 0.0);
  }
  let nrm = -sign(rd) * step(t1.yzx, t1.xyz) * step(t1.zxy, t1.xyz);
  return vec4f(max(near, 0.0), nrm);
}

struct Hit {
  t: f32,
  n: vec3f,
  plate: Plate,
  cell: vec3f,
}

// The first plate along the ray, walking the grid inside the tower's bounds, at most `steps`
// cells.
fn trace(o: vec3f, d0: vec3f, steps: i32) -> Hit {
  var hit: Hit;
  hit.t = 1e9;
  // No axis exactly zero, so every step stays finite.
  let d = select(d0, vec3f(1e-6), abs(d0) < vec3f(1e-6));
  let reach = params.girth * 1.4 * 1.6 * 1.4 + 3.0 + CELL.x;
  // Only from a little under the camera up: the camera always looks up, and what is below it is
  // let go of.
  let lo = vec3f(-reach, min(state.cam_pos.y - params.keep_below, state.top - 4.0), -reach);
  let hi = vec3f(reach, state.top + 1.0, reach);
  let mid = 0.5 * (lo + hi);
  let size = 0.5 * (hi - lo);
  var t0 = 0.0;
  if (any(abs(o - mid) > size)) {
    let b = box_hit(o, d, mid, size);
    if (b.x > 1e8) {
      return hit;
    }
    t0 = b.x + 1e-3;
  }
  let p = o + d * t0;
  var cell = floor(p / CELL);
  let stp = sign(d);
  let delta = abs(CELL / d);
  var next = ((cell + max(stp, vec3f(0.0))) * CELL - o) / d;
  var layer = 1e9;
  var squares = layer_plates(cell.y);
  for (var k = 0; k < steps; k++) {
    if (cell.y != layer) {
      layer = cell.y;
      squares = layer_plates(layer);
    }
    // Near the front the plate of the cell further in may still be flying out through this one.
    let others = select(0.0, 1.0, arriving(cell));
    for (var j = 0.0; j <= others; j += 1.0) {
      let back = way_out(cell);
      let owner = cell - vec3f(back.x, 0.0, back.y) * j;
      let pl = plate(owner, squares);
      if (pl.on > 0.5) {
        let b = box_hit(o, d, pl.centre, pl.half);
        if (b.x < hit.t) {
          hit.t = b.x;
          hit.n = b.yzw;
          hit.plate = pl;
          hit.cell = owner;
        }
      }
    }
    let t_next = min(next.x, min(next.y, next.z));
    // A plate lies inside its cell, so the first one met before leaving the cell is the nearest.
    if (hit.t <= t_next) {
      return hit;
    }
    if (next.x < next.y && next.x < next.z) {
      cell.x += stp.x;
      next.x += delta.x;
    } else if (next.y < next.z) {
      cell.y += stp.y;
      next.y += delta.y;
    } else {
      cell.z += stp.z;
      next.z += delta.z;
    }
    let at = o + d * t_next;
    if (any(abs(at - mid) > size + CELL)) {
      break;
    }
  }
  return hit;
}

// The theme now (worked out once a frame by update, see state.wgsl).
fn theme_now() -> Theme {
  return state.theme;
}

// The haze everything fades into (for concrete, the sky): brightest towards the light, deeper
// below.
fn haze(d: vec3f, th: Theme) -> vec3f {
  let toward = max(dot(d, th.sun), 0.0);
  let base = mix(th.haze_down, th.haze_up, smoothstep(-0.6, 0.4, d.y));
  return base + th.glow * (pow(toward, 4.0) * 0.7 + pow(toward, 40.0) * 0.9);
}

// The surface of a plate where a ray met it at distance t, from the tiling texture (one cell
// across): the normal bent by the cast concrete and rounded off along the plate's edges, how open
// the spot is, and the face's tone (0.5 for even). The texture's level is chosen from how much of
// it one pixel covers, since a hit is not in uniform control flow and has no derivatives.
struct Surface {
  n: vec3f,
  open: f32,
  tone: f32,
}

fn surface(p: vec3f, d: vec3f, t: f32, h: Hit, pixel: f32) -> Surface {
  let n = h.n;
  let an = abs(n);
  // The face's own axes: u and v across it, in metres.
  var tu = vec3f(0.0, 0.0, 1.0);
  var tv = vec3f(0.0, 1.0, 0.0);
  if (an.y > 0.5) {
    tu = vec3f(1.0, 0.0, 0.0);
    tv = vec3f(0.0, 0.0, 1.0);
  } else if (an.z > 0.5) {
    tu = vec3f(1.0, 0.0, 0.0);
  }
  // Placed by the plate, so a plate several cells across is one cast, its texture running on
  // across the cells.
  let uv = vec2f(dot(p, tu), dot(p, tv)) / CELL.x + vec2f(hash3(h.plate.key + 21.1), hash3(h.plate.key + 22.2));
  let across = t * pixel / max(abs(dot(n, d)), 0.15);
  let lod = log2(max(across * 512.0 / CELL.x, 1.0));
  let tex = textureSampleLevel(asset_concrete, linear_repeat, uv, lod);
  let bump = (tex.xy * 2.0 - 1.0) * params.surface;

  // The rounded edges: within `bevel` of an edge the face turns towards it, as a quarter circle.
  // Where a pixel covers more than the rounding, it is widened to the pixel and bent less, so a
  // far edge still catches a thin line of light and does not flicker.
  let q = p - h.plate.whole_c;
  let half = h.plate.whole_h;
  let r = min(params.bevel, 0.45 * min(half.x, min(half.y, half.z)));
  let wide = max(r, across * 1.5);
  let lean = select(0.0, r / wide, r > 0.0);
  let eu = clamp(1.0 - (dot(half, abs(tu)) - abs(dot(q, tu))) / wide, 0.0, 1.0) * lean;
  let ev = clamp(1.0 - (dot(half, abs(tv)) - abs(dot(q, tv))) / wide, 0.0, 1.0) * lean;
  let su = sign(dot(q, tu));
  let sv = sign(dot(q, tv));
  var bent = n * sqrt(max(1.0 - eu * eu - ev * ev, 0.0)) + tu * su * eu + tv * sv * ev;
  bent = normalize(bent + (tu * bump.x + tv * bump.y));

  var s: Surface;
  s.n = bent;
  s.open = mix(1.0, tex.z, params.surface);
  s.tone = mix(0.5, tex.w, params.surface);
  return s;
}
