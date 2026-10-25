// The wall, shared by the passes: old television sets piled up without end. The wall is cut into
// bays 2.4 m wide, and each bay into blocks 1.6 m high (shifted up or down by the bay, so the
// blocks of neighbouring bays do not line up). Each block of a bay is filled with columns of its
// own widths, and each column piled from the bottom with sets of any model narrow enough: so no
// column runs on more than a block, and no pile is all one kind. Each set sits to one side of its
// column or in the middle, and stands out by its column's depth and a little of its own. A ray walks the slots in the thin layer in front of the wall and meets at most one
// cabinet in each, exactly: nothing is marched.

// A model, designed once: its size (width, height, depth), the screen, the strip of controls,
// the speaker grille (each a rectangle, left, bottom, right, top, from the front's lower left
// corner), two knobs (centre and radius), and its style: x 1 when the strip is speaker cloth
// rather than metal, y how round the screen's corners are.
struct Model {
  size: vec3f,
  screen: vec4f,
  panel: vec4f,
  grille: vec4f,
  knob_a: vec3f,
  knob_b: vec3f,
  style: vec2f,
}

const MODELS: u32 = 7u;
const NO_GRILLE: vec4f = vec4f(-1.0, -1.0, -1.0, -1.0);

fn model_of(m: u32) -> Model {
  switch m {
    case 0u: {
      // A small portable: the controls in a strip of metal right of the screen.
      return Model(vec3f(0.46, 0.4, 0.4), vec4f(0.03, 0.04, 0.345, 0.36), vec4f(0.36, 0.035, 0.435, 0.365),
        vec4f(0.372, 0.06, 0.423, 0.2), vec3f(0.3975, 0.305, 0.021), vec3f(0.3975, 0.24, 0.015), vec2f(0.0, 0.16));
    }
    case 1u: {
      // A table set: a broad strip of brushed metal beside the screen.
      return Model(vec3f(0.66, 0.54, 0.5), vec4f(0.04, 0.05, 0.5, 0.49), vec4f(0.52, 0.04, 0.625, 0.5),
        vec4f(0.535, 0.07, 0.61, 0.27), vec3f(0.5725, 0.43, 0.028), vec3f(0.5725, 0.35, 0.021), vec2f(0.0, 0.16));
    }
    case 2u: {
      // A console: the controls in a strip under a wide screen.
      return Model(vec3f(0.86, 0.66, 0.6), vec4f(0.05, 0.16, 0.81, 0.62), vec4f(0.045, 0.03, 0.815, 0.135),
        vec4f(0.065, 0.045, 0.5, 0.12), vec3f(0.62, 0.0825, 0.03), vec3f(0.72, 0.0825, 0.03), vec2f(0.0, 0.16));
    }
    case 3u: {
      // A little monitor: nearly all screen, two small knobs on a strip below.
      return Model(vec3f(0.34, 0.32, 0.34), vec4f(0.025, 0.07, 0.315, 0.3), vec4f(0.02, 0.012, 0.32, 0.058),
        NO_GRILLE, vec3f(0.235, 0.035, 0.012), vec3f(0.285, 0.035, 0.012), vec2f(0.0, 0.08));
    }
    case 4u: {
      // A wooden set: speaker cloth left of the screen, the knobs under it.
      return Model(vec3f(0.8, 0.56, 0.5), vec4f(0.3, 0.05, 0.77, 0.51), vec4f(0.03, 0.18, 0.27, 0.51),
        NO_GRILLE, vec3f(0.09, 0.1, 0.032), vec3f(0.21, 0.1, 0.032), vec2f(1.0, 0.2));
    }
    case 5u: {
      // A studio monitor: square corners, a strip of controls along the bottom.
      return Model(vec3f(0.58, 0.5, 0.52), vec4f(0.04, 0.12, 0.54, 0.47), vec4f(0.03, 0.02, 0.55, 0.095),
        vec4f(0.04, 0.035, 0.2, 0.08), vec3f(0.44, 0.0575, 0.016), vec3f(0.5, 0.0575, 0.016), vec2f(0.0, 0.06));
    }
    default: {
      // A tall fifties set: a round-cornered screen on top, the knobs, the speaker cloth below.
      return Model(vec3f(0.52, 0.7, 0.48), vec4f(0.05, 0.3, 0.47, 0.66), vec4f(0.07, 0.04, 0.45, 0.2),
        NO_GRILLE, vec3f(0.17, 0.25, 0.025), vec3f(0.35, 0.25, 0.025), vec2f(1.0, 0.3));
    }
  }
}

// The models from narrowest to widest.
fn by_width(i: u32) -> u32 {
  return array<u32, 7>(3u, 0u, 6u, 5u, 1u, 4u, 2u)[min(i, 6u)];
}

fn width_of(m: u32) -> f32 {
  return model_of(m).size.x;
}

const BAY: f32 = 2.4;
const BLOCK: f32 = 1.6;
// The gap beside a set at least, and between one in a pile and the next.
const SIDE_GAP: f32 = 0.04;
const STACK_GAP: f32 = 0.012;
// How far in front of the wall the deepest set reaches.
const DEEPEST: f32 = 0.95;

// A block of a bay: its columns, where each starts (x), the widest model it takes (an index into
// by_width), and how many there are. The rest of the bay's width is shared out as gaps.
struct Bay {
  x: array<f32, 6>,
  widest: array<u32, 5>,
  count: u32,
}

// How far bay b's blocks are shifted up.
fn bay_lift(b: f32) -> f32 {
  return hash(vec2f(b, 3.7)) * BLOCK;
}

fn bay_of(b: f32, k: f32) -> Bay {
  let seed = b * 7.3 + k * 1.71;
  var bay: Bay;
  var widths: array<f32, 5>;
  var used = 0.0;
  var n = 0u;
  for (var j = 0u; j < 5u; j++) {
    // Narrow columns more often than wide ones.
    var c = u32(pow(hash(vec2f(seed, f32(j) + 0.5)), 1.25) * 7.0);
    // Narrower until it fits.
    loop {
      if (c == 0u || used + width_of(by_width(c)) + SIDE_GAP <= BAY) {
        break;
      }
      c -= 1u;
    }
    let w = width_of(by_width(c)) + SIDE_GAP;
    if (used + w > BAY) {
      break;
    }
    bay.widest[j] = c;
    widths[j] = w;
    used += w;
    n++;
  }
  // What is left over widens columns a model at a time, from a random column on, so the gaps
  // between them stay narrow.
  let first = u32(hash(vec2f(seed, 7.7)) * 5.0);
  for (var round = 0u; round < 2u; round++) {
    for (var k = 0u; k < 5u; k++) {
      let j = (first + k) % 5u;
      if (j < n && bay.widest[j] < MODELS - 1u) {
        let grow = width_of(by_width(bay.widest[j] + 1u)) - width_of(by_width(bay.widest[j]));
        if (used + grow <= BAY) {
          bay.widest[j] += 1u;
          widths[j] += grow;
          used += grow;
        }
      }
    }
  }
  let spare = (BAY - used) / f32(max(n, 1u));
  var x = b * BAY;
  for (var j = 0u; j < 5u; j++) {
    bay.x[j] = x;
    if (j < n) {
      x += widths[j] + spare;
    }
  }
  bay.x[5] = x;
  bay.x[n] = (b + 1.0) * BAY;
  bay.count = n;
  return bay;
}

// One column of a block: its slot's left and right edge, the widest model it takes, its number
// on the wall, the block's bottom and how far it stands out from the wall.
struct Column {
  x0: f32,
  x1: f32,
  widest: u32,
  id: f32,
  y0: f32,
  push: f32,
}

fn column_in(bay_in: Bay, b: f32, k: f32, x: f32) -> Column {
  // (A copy that can be indexed at run time.)
  var bay = bay_in;
  var j = 0u;
  for (var k = 1u; k < 5u; k++) {
    if (k < bay.count && x >= bay.x[k]) {
      j = k;
    }
  }
  var c: Column;
  c.x0 = bay.x[j];
  c.x1 = bay.x[j + 1u];
  c.widest = bay.widest[j];
  c.id = b * 8.0 + f32(j) + k * 1009.0;
  c.y0 = k * BLOCK + bay_lift(b);
  // Three depths: flush, a hand forward, a stand forward.
  let h = hash(vec2f(c.id, 5.1));
  c.push = select(select(0.0, 0.1, h > 0.45), 0.25, h > 0.8);
  return c;
}

fn column_at(x: f32, y: f32) -> Column {
  let b = floor(x / BAY);
  let k = floor((y - bay_lift(b)) / BLOCK);
  return column_in(bay_of(b, k), b, k, x);
}

// A column's pile: where each set's slot starts (the last entry is where the pile ends) and its
// model, from the bottom up; and how much taller than its model each set's foot is, so the pile
// fills its block and no gap runs along the blocks' seams.
struct Pile {
  y: array<f32, 8>,
  model: array<u32, 7>,
  count: u32,
  plinth: f32,
}

fn pile_of(c: Column) -> Pile {
  var pile: Pile;
  var y = c.y0;
  let top = y + BLOCK;
  var n = 0u;
  for (var i = 0u; i < 7u; i++) {
    // A model the column is wide enough for, mostly one about as wide as the column, so the pile
    // is close; when it is too tall for what is left, the tallest that fits, else the pile ends.
    let narrower = u32(pow(hash(vec2f(c.id * 1.3, f32(i) + 0.25)), 2.5) * f32(c.widest + 1u));
    var m = by_width(c.widest - min(narrower, c.widest));
    if (y + model_of(m).size.y + STACK_GAP > top) {
      var best = 99u;
      var tallest = 0.0;
      for (var w = 0u; w <= c.widest; w++) {
        let h = model_of(by_width(w)).size.y;
        if (y + h + STACK_GAP <= top && h > tallest) {
          tallest = h;
          best = by_width(w);
        }
      }
      if (best == 99u) {
        break;
      }
      m = best;
    }
    pile.y[i] = y;
    pile.model[i] = m;
    y += model_of(m).size.y + STACK_GAP;
    n++;
  }
  pile.count = n;
  pile.plinth = select(0.0, (top - y) / f32(n), n > 0u);
  y = c.y0;
  for (var i = 0u; i < 8u; i++) {
    pile.y[i] = y;
    if (i < n) {
      y += model_of(pile.model[i]).size.y + pile.plinth + STACK_GAP;
    }
  }
  return pile;
}

// A set: the cabinet's box (centre and half size), its model, its number (column, height),
// whether there is one in the slot, the slot's bottom and top (for the walk), and how much taller
// its foot is than its model's.
struct Set {
  centre: vec3f,
  half: vec3f,
  model: u32,
  id: vec2f,
  on: bool,
  y0: f32,
  y1: f32,
  plinth: f32,
}

// The set in column c at height y.
fn set_at(c: Column, pile_in: Pile, y: f32) -> Set {
  var pile = pile_in;
  var s: Set;
  s.on = false;
  s.y0 = c.y0;
  s.y1 = s.y0 + BLOCK;
  var i = 0u;
  for (var j = 0u; j < 7u; j++) {
    if (j < pile.count && y >= pile.y[j]) {
      i = j;
    }
  }
  if (pile.count == 0u || y < pile.y[0]) {
    return s;
  }
  if (y >= pile.y[pile.count]) {
    // Above the pile: an empty shelf to the block's top.
    s.y0 = pile.y[pile.count];
    return s;
  }
  s.y0 = pile.y[i];
  s.y1 = pile.y[i + 1u];
  s.model = pile.model[i];
  let m = model_of(s.model);
  s.id = vec2f(c.id, floor(s.y0 * 7.0));
  // A few slots are empty.
  s.on = hash(s.id + 0.37) >= params.gaps;
  // To the left, the middle or the right of the column.
  let room = c.x1 - c.x0 - SIDE_GAP - m.size.x;
  let side = floor(hash(s.id + 0.71) * 3.0) * 0.5;
  let x = c.x0 + 0.5 * SIDE_GAP + room * side + 0.5 * m.size.x;
  let depth = m.size.z + c.push + 0.08 * hash(s.id + 0.93);
  let height = m.size.y + pile.plinth;
  s.plinth = pile.plinth;
  s.centre = vec3f(x, s.y0 + 0.5 * height, -0.5 * depth);
  s.half = vec3f(0.5 * m.size.x, 0.5 * height, 0.5 * depth);
  return s;
}

fn set_at_point(x: f32, y: f32) -> Set {
  let c = column_at(x, y);
  return set_at(c, pile_of(c), y);
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
    return vec4f(1e9, 0.0, 0.0, -1.0);
  }
  let nrm = -sign(rd) * step(t1.yzx, t1.xyz) * step(t1.zxy, t1.xyz);
  return vec4f(max(near, 0.0), nrm);
}

// What a ray met: 0 nothing, 1 a cabinet, 2 the wall behind.
struct Hit {
  t: f32,
  n: vec3f,
  kind: u32,
  tv: Set,
}

// The first thing along the ray, walking at most `steps` slots in the layer in front of the wall.
fn trace(o: vec3f, d0: vec3f, steps: i32) -> Hit {
  var hit: Hit;
  hit.t = 1e9;
  hit.kind = 0u;
  // No axis exactly zero, so every step stays finite.
  let d = select(d0, vec3f(1e-6), abs(d0) < vec3f(1e-6));
  // The layer between z = -DEEPEST and the wall.
  let ta = (-DEEPEST - o.z) / d.z;
  let tb = -o.z / d.z;
  var t = max(min(ta, tb), 0.0);
  let t_end = max(ta, tb);
  if (t_end <= 0.0) {
    return hit;
  }
  var bay_key = vec2f(1e9);
  var bay: Bay;
  var pile_key = 1e9;
  var pile: Pile;
  for (var k = 0; k < steps; k++) {
    if (t >= t_end) {
      break;
    }
    let p = o + d * (t + 1e-4);
    let b = floor(p.x / BAY);
    let kb = floor((p.y - bay_lift(b)) / BLOCK);
    if (any(vec2f(b, kb) != bay_key)) {
      bay_key = vec2f(b, kb);
      bay = bay_of(b, kb);
    }
    let col = column_in(bay, b, kb, p.x);
    if (col.id != pile_key) {
      pile_key = col.id;
      pile = pile_of(col);
    }
    let s = set_at(col, pile, p.y);
    if (s.on) {
      let h = box_hit(o, d, s.centre, s.half);
      if (h.x < 1e8) {
        hit.t = h.x;
        hit.n = h.yzw;
        hit.kind = 1u;
        hit.tv = s;
        return hit;
      }
    }
    // On to the next slot, across the column's side or the slot's top or bottom.
    let tx = (select(col.x0, col.x1, d.x > 0.0) - o.x) / d.x;
    let ty = (select(s.y0, s.y1, d.y > 0.0) - o.y) / d.y;
    t = max(min(tx, ty), t + 1e-4);
  }
  // Past every cabinet: the wall, if the ray was going towards it and got there.
  if (d.z > 0.0 && t >= t_end) {
    hit.t = tb;
    hit.n = vec3f(0.0, 0.0, -1.0);
    hit.kind = 2u;
  }
  return hit;
}
