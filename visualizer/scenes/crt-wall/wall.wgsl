// The wall for the passes (its models, bays, columns, piles and sets are in state.wgsl): a ray
// walks the slots in the thin layer in front of the wall and meets at most one cabinet in each,
// exactly: nothing is marched; and the plan of it worked out once a frame.

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
