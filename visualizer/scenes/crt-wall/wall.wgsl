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
const BY_WIDTH = array<u32, 7>(3u, 0u, 6u, 5u, 1u, 4u, 2u);

fn by_width(i: u32) -> u32 {
  return BY_WIDTH[min(i, 6u)];
}

// The models' sizes as in model_of, in a table: the walk needs only these, often.
const SIZES = array<vec3f, 7>(vec3f(0.46, 0.4, 0.4), vec3f(0.66, 0.54, 0.5), vec3f(0.86, 0.66, 0.6),
  vec3f(0.34, 0.32, 0.34), vec3f(0.8, 0.56, 0.5), vec3f(0.58, 0.5, 0.52), vec3f(0.52, 0.7, 0.48));

fn size_of(m: u32) -> vec3f {
  return SIZES[min(m, 6u)];
}

fn width_of(m: u32) -> f32 {
  return size_of(m).x;
}

// The widths and heights of the models from narrowest to widest (of by_width), in tables.
const WIDTH_BY_RANK = array<f32, 7>(0.34, 0.46, 0.52, 0.58, 0.66, 0.8, 0.86);
const HEIGHT_BY_RANK = array<f32, 7>(0.32, 0.4, 0.7, 0.5, 0.54, 0.56, 0.66);

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
    // Narrower until it fits (the widths grow, so what fits is all up to the widest that does).
    var fits = 0u;
    for (var r = 1u; r < 7u; r++) {
      if (used + WIDTH_BY_RANK[r] + SIDE_GAP <= BAY) {
        fits = r;
      }
    }
    c = min(c, fits);
    let w = WIDTH_BY_RANK[c] + SIDE_GAP;
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
    // Columns first, first + 1, ... round to first - 1: as kk runs over two rounds of the five.
    for (var kk = 0u; kk < 10u; kk++) {
      let j = kk % 5u;
      if (kk >= first && kk < first + 5u && j < n && bay.widest[j] < MODELS - 1u) {
        let grow = WIDTH_BY_RANK[bay.widest[j] + 1u] - WIDTH_BY_RANK[bay.widest[j]];
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
  for (var j = 0u; j < 6u; j++) {
    if (j == n) {
      bay.x[j] = (b + 1.0) * BAY;
    }
  }
  bay.count = n;
  return bay;
}

// One column of a block: its slot's left and right edge, the widest model it takes, its number
// on the wall, the block's bottom and how far it stands out from the wall.
struct Column {
  x0: f32,
  x1: f32,
  widest: u32,
  j: u32,
  id: f32,
  y0: f32,
  push: f32,
}

fn column_in(bay_in: Bay, b: f32, k: f32, x: f32) -> Column {
  var bay = bay_in;
  var j = 0u;
  var c: Column;
  c.x0 = bay.x[0];
  c.x1 = bay.x[1];
  c.widest = bay.widest[0];
  for (var k = 1u; k < 5u; k++) {
    if (k < bay.count && x >= bay.x[k]) {
      j = k;
      c.x0 = bay.x[k];
      c.x1 = bay.x[k + 1u];
      c.widest = bay.widest[k];
    }
  }
  c.j = j;
  c.id = b * 8.0 + f32(j) + k * 1009.0;
  c.y0 = k * BLOCK + bay_lift(b);
  // Three depths: flush, a hand forward, a stand forward.
  let h = hash(vec2f(c.id, 5.1));
  c.push = select(select(0.0, 0.1, h > 0.45), 0.25, h > 0.8);
  return c;
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
    if (y + size_of(m).y + STACK_GAP > top) {
      var best = 99u;
      var tallest = 0.0;
      for (var w = 0u; w < 7u; w++) {
        let h = HEIGHT_BY_RANK[w];
        if (w <= c.widest && y + h + STACK_GAP <= top && h > tallest) {
          tallest = h;
          best = BY_WIDTH[w];
        }
      }
      if (best == 99u) {
        break;
      }
      m = best;
    }
    pile.model[i] = m;
    y += size_of(m).y + STACK_GAP;
    n++;
  }
  pile.count = n;
  pile.plinth = select(0.0, (top - y) / f32(n), n > 0u);
  y = c.y0;
  for (var i = 0u; i < 8u; i++) {
    pile.y[i] = y;
    if (i < n) {
      y += size_of(pile.model[i]).y + pile.plinth + STACK_GAP;
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
  var y0 = pile.y[0];
  var y1 = pile.y[1];
  var model = pile.model[0];
  for (var j = 1u; j < 7u; j++) {
    if (j < pile.count && y >= pile.y[j]) {
      y0 = pile.y[j];
      y1 = pile.y[j + 1u];
      model = pile.model[j];
    }
  }
  var end = pile.y[0];
  for (var j = 1u; j < 8u; j++) {
    if (j == pile.count) {
      end = pile.y[j];
    }
  }
  if (pile.count == 0u || y < pile.y[0]) {
    return s;
  }
  if (y >= end) {
    // Above the pile: an empty shelf to the block's top.
    s.y0 = end;
    return s;
  }
  s.y0 = y0;
  s.y1 = y1;
  s.model = model;
  let m = size_of(s.model);
  s.id = vec2f(c.id, floor(s.y0 * 7.0));
  // A few slots are empty.
  s.on = hash(s.id + 0.37) >= params.gaps;
  // To the left, the middle or the right of the column.
  let room = c.x1 - c.x0 - SIDE_GAP - m.x;
  let side = floor(hash(s.id + 0.71) * 3.0) * 0.5;
  let x = c.x0 + 0.5 * SIDE_GAP + room * side + 0.5 * m.x;
  let depth = m.z + c.push + 0.08 * hash(s.id + 0.93);
  let height = m.y + pile.plinth;
  s.plinth = pile.plinth;
  s.centre = vec3f(x, s.y0 + 0.5 * height, -0.5 * depth);
  s.half = vec3f(0.5 * m.x, 0.5 * height, 0.5 * depth);
  return s;
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

// ---- The plan: worked out once a frame (by plan.wgsl) around the spot on the wall the camera
// looks at, for the passes to read rather than work out again for every pixel. Whoever reads it
// first checks it is big enough (it is not, at the smallest internal resolutions), and works out
// itself what it does not hold. From the top:
// - PLAN_ROWS rows, one for each block of a bay in a window PLAN_BAYS wide and PLAN_BLOCKS high:
//   its key (bay, block, 1), the bay (3 texels), then each column's pile (5 texels);
// - PLAN_POOLS rows of as many cells of the pools of light, each where it is and how strong; in
//   the first of them, after its cells, the first cell's number and where the window of bays
//   starts (each with a 1 after it);
// - two rows of the points of the scopes' stereo picture, two a texel, and a row of the box
//   around each run of eight of its segments (the last run seven). ----

const PLAN_BAYS: u32 = 8u;
const PLAN_BLOCKS: u32 = 8u;
const PLAN_ROWS: u32 = 64u;
const PLAN_POOLS: u32 = 8u;
const PLAN_SCOPE: u32 = PLAN_ROWS + PLAN_POOLS;
const PLAN_WIDTH: u32 = 29u;
const PLAN_HEIGHT: u32 = PLAN_SCOPE + 3u;

// The key light falls in pools along the wall, as from lamps overhead, dark between them: a pool
// every few metres, each its own strength. A cell's pool: where it is, and how strong.
const POOL_CELL: vec2f = vec2f(3.2, 2.8);

fn pool_of(cell: vec2f) -> vec3f {
  let at = (cell + 0.5 + (vec2f(hash(cell + 0.17), hash(cell + 0.59)) - 0.5) * 0.6) * POOL_CELL;
  return vec3f(at, 0.6 + 0.6 * hash(cell + 0.83));
}

// The scopes' stereo picture: left against right, turned so that mono stands upright.
const POINTS: i32 = 96;
const SCOPE_RUNS: i32 = 12;

fn stereo_point(i: i32) -> vec2f {
  let s = rv_waveform(f32(i) / f32(POINTS - 1)) * state.gain;
  return vec2f(s.x - s.y, s.x + s.y) * 0.62;
}

// The spot on the wall the camera looks at.
fn plan_look() -> vec2f {
  let f = vec3f(sin(state.yaw) * cos(state.pitch), sin(state.pitch), cos(state.yaw) * cos(state.pitch));
  return state.cam.xy + f.xy * ((-0.5 - state.cam.z) / max(f.z, 0.05));
}

// The bay the window starts at, and the height its blocks are counted from (before each bay's
// own lift).
fn plan_corner() -> vec2f {
  let at = plan_look();
  return vec2f(floor(at.x / BAY) - f32(PLAN_BAYS / 2u), at.y);
}

fn plan_first_pool() -> vec2f {
  return floor(plan_look() / POOL_CELL) - f32(PLAN_POOLS / 2u);
}

// The block of a bay a row is for.
fn plan_key(row: u32) -> vec2f {
  let corner = plan_corner();
  let b = corner.x + f32(row % PLAN_BAYS);
  let k = floor((corner.y - bay_lift(b)) / BLOCK) - f32(PLAN_BLOCKS / 2u) + f32(row / PLAN_BAYS);
  return vec2f(b, k);
}

// The row of block k of bay b (lifted by `lift`) in a plan whose window starts at `corner`, or -1
// outside it. Whoever reads the row checks its key: the lift is worked out again here and may
// round differently.
fn plan_row(b: f32, k: f32, lift: f32, corner: vec2f) -> i32 {
  let bi = b - corner.x;
  let ki = k - (floor((corner.y - lift) / BLOCK) - f32(PLAN_BLOCKS / 2u));
  if (bi < 0.0 || bi >= f32(PLAN_BAYS) || ki < 0.0 || ki >= f32(PLAN_BLOCKS)) {
    return -1;
  }
  return i32(bi) + i32(ki) * i32(PLAN_BAYS);
}

// Column j of a block of a bay (found at its left edge).
fn column_j(bay_in: Bay, b: f32, k: f32, j: u32) -> Column {
  var bay = bay_in;
  var x = bay.x[0];
  for (var i = 1u; i < 5u; i++) {
    if (i == j) {
      x = bay.x[i];
    }
  }
  return column_in(bay, b, k, x);
}

// The bay and pile as texels, and back.
fn bay_texel(bay_in: Bay, t: u32) -> vec4f {
  var bay = bay_in;
  if (t == 0u) {
    return vec4f(bay.x[0], bay.x[1], bay.x[2], bay.x[3]);
  }
  if (t == 1u) {
    return vec4f(bay.x[4], bay.x[5], f32(bay.widest[0]), f32(bay.widest[1]));
  }
  return vec4f(f32(bay.widest[2]), f32(bay.widest[3]), f32(bay.widest[4]), f32(bay.count));
}

fn bay_from(t0: vec4f, t1: vec4f, t2: vec4f) -> Bay {
  var bay: Bay;
  bay.x[0] = t0.x;
  bay.x[1] = t0.y;
  bay.x[2] = t0.z;
  bay.x[3] = t0.w;
  bay.x[4] = t1.x;
  bay.x[5] = t1.y;
  bay.widest[0] = u32(t1.z);
  bay.widest[1] = u32(t1.w);
  bay.widest[2] = u32(t2.x);
  bay.widest[3] = u32(t2.y);
  bay.widest[4] = u32(t2.z);
  bay.count = u32(t2.w);
  return bay;
}

fn pile_texel(pile_in: Pile, t: u32) -> vec4f {
  var pile = pile_in;
  switch t {
    case 0u: {
      return vec4f(pile.y[0], pile.y[1], pile.y[2], pile.y[3]);
    }
    case 1u: {
      return vec4f(pile.y[4], pile.y[5], pile.y[6], pile.y[7]);
    }
    case 2u: {
      return vec4f(f32(pile.model[0]), f32(pile.model[1]), f32(pile.model[2]), f32(pile.model[3]));
    }
    case 3u: {
      return vec4f(f32(pile.model[4]), f32(pile.model[5]), f32(pile.model[6]), f32(pile.count));
    }
    default: {
      return vec4f(pile.plinth, 0.0, 0.0, 0.0);
    }
  }
}

fn pile_from(t0: vec4f, t1: vec4f, t2: vec4f, t3: vec4f, t4: vec4f) -> Pile {
  var pile: Pile;
  pile.y[0] = t0.x;
  pile.y[1] = t0.y;
  pile.y[2] = t0.z;
  pile.y[3] = t0.w;
  pile.y[4] = t1.x;
  pile.y[5] = t1.y;
  pile.y[6] = t1.z;
  pile.y[7] = t1.w;
  pile.model[0] = u32(t2.x);
  pile.model[1] = u32(t2.y);
  pile.model[2] = u32(t2.z);
  pile.model[3] = u32(t2.w);
  pile.model[4] = u32(t3.x);
  pile.model[5] = u32(t3.y);
  pile.model[6] = u32(t3.z);
  pile.count = u32(t3.w);
  pile.plinth = t4.x;
  return pile;
}
