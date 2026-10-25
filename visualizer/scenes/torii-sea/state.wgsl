// What Torii Sea keeps from frame to frame, and what the update and the passes share: the line
// the reef winds along. x runs across, y up, z along the reef, in metres.

struct State {
  // The camera: where it is, which way it looks, its roll and its lens (1, less to zoom in).
  cam_pos: vec3f,
  roll: f32,
  cam_fwd: vec3f,
  lens: f32,
  // Gliding: how far along, speed, where the speed heads, and the pace the music sets.
  s: f32,
  vel: f32,
  vtarget: f32,
  tempo: f32,
  // How fast the music sounds (smoothed), its energy (and that smoothed again) and the kick.
  feel: f32,
  energy: f32,
  ens: f32,
  kick: f32,
  // How loud it is this moment within the song (0 to 1, smoothed a little): what drives how hard
  // the spirits move.
  moment: f32,
  // The shot (1 from behind and above, 2 from behind and to one side, low), when it last
  // changed, and (in lat) the side of the line the second is on.
  view: f32,
  last_view: f32,
  lat: f32,
  // The sea: how tall the waves are, and the swell an accent sends (and where it is).
  wave: f32,
  swell: f32,
  swell_phase: f32,
  // The sea holds still between changes; each change is a new set of triangles: its number, how
  // tall its waves stand, and when it came.
  sea_gen: f32,
  sea_amp: f32,
  sea_last: f32,
  last_accent: f32,
  last_energy: f32,
  // The theme colour's hue (OKLab, radians): the player's accent colour, or the one set. The
  // spirits take it, and the sea a little of it; the torii keep their own.
  theme_hue: f32,
  // The light: fading from one to the next, the one waiting, and when it last changed.
  light_from: f32,
  light_to: f32,
  light_mix: f32,
  light_next: f32,
  last_light: f32,
  // The beat: torii passed per beat, how locked the glide is to the beat, the time since the
  // last kick (for the rings on the water).
  per_beat: f32,
  lock: f32,
  since_kick: f32,
  // The spirits: the beat's surge, the live levels of the bass, mids and treble, the snare's
  // flick, a leap into the sky (its phase, 0 to 1, and strength), how they move (0 braid,
  // 1 snake, 2 fan), when that last changed, and how many fly.
  surge: f32,
  levels: vec3f,
  flick: f32,
  leap: f32,
  leap_power: f32,
  mode: f32,
  last_mode: f32,
  count: f32,
  // The path ring: the slot written last and the time until the next; how many points have been
  // written in all (the ribbon's twist and folds stay with the point they were made at).
  slot: u32,
  until: f32,
  written: f32,
  // The camera's frame, for projecting the spirits.
  cam_right: vec3f,
  cam_up: vec3f,
  // Each spirit: position and how present it is (w); velocity.
  spirit: array<vec4f, 5>,
  spirit_vel: array<vec4f, 5>,
  // Each spirit's outing away from the others: where it goes (as an offset from its place) and
  // how far through it is (w, 0 when at home); and how long it lasts.
  trip: array<vec4f, 5>,
  trip_len: array<vec4f, 5>,
  // Path points in the world, SPIRITS × POINTS, by ring slot.
  path: array<vec4f, 200>,
  // The paper ribbon each spirit trails, newest point first, on the screen: its two edges (x, y
  // of each); their depths, 1 if both are in front of the camera, and the point's age (0 at the
  // head, 1 at the tail end). Then, for the stretch from each point to the next, the world
  // normals of its two triangles.
  ribbon: array<vec4f, 200>,
  ribbon_z: array<vec4f, 200>,
  ribbon_n: array<vec4f, 400>,
  // The flakes shed from the ribbon, one per point at most: on the screen (x, y, depth, and its
  // turn on the screen); its world normal, and its size on the screen (0: none).
  flake: array<vec4f, 200>,
  flake_n: array<vec4f, 200>,
  // Each spirit's box on the screen, ribbon and flakes and all; and the boxes of its ribbon in
  // eight stretches of five points (and their flakes), so a pixel skips most of it at once.
  bounds: array<vec4f, 5>,
  chunks: array<vec4f, 40>,
  // The torii in sight, worked out once a frame (see update), nearest first: each one's numbers
  // packed in three rows (see pack_gate); how many; and the screen's tiles, each listing the
  // torii whose bounding sphere covers it, nearest first, so a pixel tests only those.
  torii_a: array<vec4f, 384>,
  torii_b: array<vec4f, 384>,
  torii_c: array<vec4f, 384>,
  torii_n: u32,
  tile_n: array<u32, 576>,
  tile: array<u32, 27648>,
}

// The most torii in sight at once; the screen's tiles, and how many torii a tile lists at most.
const TORII: u32 = 384u;
const TX: u32 = 32u;
const TY: u32 = 18u;
const TILES: u32 = 576u;
const LIST: u32 = 48u;

const SPIRITS: u32 = 5u;
const POINTS: u32 = 40u;
// Points in each stretch of a ribbon that is boxed on its own.
const CHUNK: u32 = 5u;

// ---- Colour, in OKLab (where a step in lightness or hue looks as big as it is) ----

fn oklab(c: vec3f) -> vec3f {
  let l = pow(max(0.4122214708 * c.r + 0.5363325363 * c.g + 0.0514459929 * c.b, 0.0), 1.0 / 3.0);
  let m = pow(max(0.2119034982 * c.r + 0.6806995451 * c.g + 0.1073969566 * c.b, 0.0), 1.0 / 3.0);
  let s = pow(max(0.0883024619 * c.r + 0.2817188376 * c.g + 0.6299787005 * c.b, 0.0), 1.0 / 3.0);
  return vec3f(0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s, 1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s, 0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s);
}

// A colour (linear) from OKLCH: lightness, chroma (times `colourful`), hue in radians.
fn oklch(lightness: f32, chroma: f32, hue: f32) -> vec3f {
  let a = chroma * params.colourful * cos(hue);
  let b = chroma * params.colourful * sin(hue);
  let l = pow(lightness + 0.3963377774 * a + 0.2158037573 * b, 3.0);
  let m = pow(lightness - 0.1055613458 * a - 0.0638541728 * b, 3.0);
  let s = pow(lightness - 0.0894841775 * a - 1.2914855480 * b, 3.0);
  return max(vec3f(4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s, -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s, -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s), vec3f(0.0));
}

// Colour c at the theme's hue, keeping how light it is, as colourful as `chroma`.
fn themed(c: vec3f, chroma: f32) -> vec3f {
  return oklch(oklab(c).x, chroma, state.theme_hue);
}

fn hash(p: vec2f) -> f32 {
  var p3 = fract(vec3f(p.x, p.y, p.x) * 0.1031);
  p3 += dot(p3, p3.yzx + 33.33);
  return fract((p3.x + p3.y) * p3.z);
}

// Where the reef runs: its centre across at z, and how fast that changes.
fn path_x(z: f32) -> f32 {
  let w = params.winding;
  return w * (7.0 * sin(z * 0.011) + 3.5 * sin(z * 0.027 + 1.3) + 1.2 * sin(z * 0.061 + 0.4));
}

fn path_dx(z: f32) -> f32 {
  let w = params.winding;
  return w * (7.0 * 0.011 * cos(z * 0.011) + 3.5 * 0.027 * cos(z * 0.027 + 1.3) + 1.2 * 0.061 * cos(z * 0.061 + 0.4));
}

// ---- The torii, where each stands (shared, so the camera can thread them) ----

// A torii: its kind (0 vermilion, 1 stone, 2 old and faded), how broken it is (0 whole, 1 its
// top fallen, 2 two stumps), its size (1 for one 6 m tall), where it stands (its centre across,
// its slot along, the height of its feet: below the water it is sunk), its lean, and whether
// there is one at all.
struct Gate {
  kind: f32,
  broken: f32,
  size: f32,
  x: f32,
  z: f32,
  base: f32,
  lean: f32,
  here: f32,
  // Turned on its base away from facing along the line, and its kasagi slipped (one end down).
  turn: f32,
  askew: f32,
  // Seen so small (a few dozen pixels at most) that it is drawn plainly: a few boxes, no lanterns.
  lod: f32,
}

// A torii's numbers in three rows, with its slot (its slot along the line, or -1 - j for stray
// j), and back. (Only those there are packed: b.w, where `here` would be, holds how near the
// camera any of it is.)
fn unpack_gate(a: vec4f, b: vec4f, c: vec4f) -> Gate {
  var g: Gate;
  g.kind = a.x;
  g.broken = a.y;
  g.size = a.z;
  g.x = a.w;
  g.z = b.x;
  g.base = b.y;
  g.lean = b.z;
  g.here = 1.0;
  g.turn = c.x;
  g.askew = c.y;
  g.lod = c.z;
  return g;
}

fn kind_of(i: f32, salt: f32) -> f32 {
  let r = hash(vec2f(i, 3.7 + salt));
  return select(select(0.0, 2.0, r < params.stone + params.old), 1.0, r < params.stone);
}

// Where slot i's torii of the line stands along it: about every `spacing` metres, each shifted
// its own way by up to half of `line_jitter` of that, so a gap is within `line_jitter` of the
// spacing either way (10 m at 0.3: 7 to 13 m).
fn gate_z(i: f32) -> f32 {
  return (i + (hash(vec2f(i, 33.1)) - 0.5) * params.line_jitter) * params.spacing;
}

// How far along the line z is, counted in gates by where they really stand: the slot of the gate
// behind z, plus how far on towards the next (0 to 1).
fn gate_pos(z: f32) -> f32 {
  var i = floor(z / params.spacing);
  if (gate_z(i) > z) {
    i -= 1.0;
  } else if (gate_z(i + 1.0) <= z) {
    i += 1.0;
  }
  let a = gate_z(i);
  let b = gate_z(i + 1.0);
  return i + clamp((z - a) / max(b - a, 1e-3), 0.0, 1.0);
}

// The torii in slot i of the line, about one every `spacing` metres, big and small: mostly whole, but
// some broken (the kasagi fallen, or only stumps left) and some crooked (leaning, turned on its
// base, its kasagi slipped).
fn gate(i: f32) -> Gate {
  var g: Gate;
  g.kind = kind_of(i, 0.0);
  g.broken = 0.0;
  g.size = mix(0.85, 1.5, pow(hash(vec2f(i, 6.6)), 1.3));
  g.z = gate_z(i);
  g.x = path_x(g.z) + (hash(vec2f(i, 2.2)) - 0.5) * 2.0 * params.scatter;
  g.base = -mix(0.3, 1.8, hash(vec2f(i, 1.3)));
  // The nuki stays clear of the camera passing under it.
  g.base = max(g.base, params.through + 1.0 - 4.28 * g.size);
  g.lean = (hash(vec2f(i, 4.8)) - 0.5) * 0.04;
  g.turn = 0.0;
  g.askew = 0.0;
  let r = hash(vec2f(i, 21.3));
  if (r < params.line_crooked) {
    g.lean = (hash(vec2f(i, 22.1)) - 0.5) * 0.36;
    g.turn = (hash(vec2f(i, 23.4)) - 0.5) * 0.6;
    g.askew = (hash(vec2f(i, 24.2)) - 0.5) * 0.3;
  }
  let b = hash(vec2f(i, 25.7));
  // Ruins, 1 snapped, 2 top fallen, 3 stumps (see part_hits).
  g.broken = select(select(select(0.0, 1.0, b < params.line_ruin), 2.0, b < params.line_ruin * 0.6), 3.0, b < params.line_ruin * 0.25);
  let grand = params.grand_every > 0.5 && i - params.grand_every * floor(i / params.grand_every) < 0.5;
  if (grand) {
    g.size = 1.8;
    g.kind = 0.0;
    g.broken = 0.0;
    g.base = -0.6;
    g.x = path_x(g.z);
    g.lean = 0.0;
    g.turn = 0.0;
    g.askew = 0.0;
  }
  g.here = f32(hash(vec2f(i, 5.3)) > params.missing || grand);
  return g;
}

// The stray torii, out in the sea on either side of the line, two to a slot (j = 2 × slot +
// side): ruins, mostly, their tops fallen or only stumps left, leaning, often half sunk, often
// faded.
// Where stray j stands, its size and whether it is there at all (x, z, size, here): all that is
// needed to pass it by.
fn stray_place(j: f32) -> vec4f {
  let i = floor(j * 0.5);
  let side = select(-1.0, 1.0, j - 2.0 * i > 0.5);
  let size = mix(0.6, 1.6, hash(vec2f(j, 16.6)));
  // Anywhere along its slot, and out from just clear of the line to `stray_reach` metres, most of
  // them nearer in.
  let z = (i + hash(vec2f(j, 12.7)) - 0.5) * params.spacing;
  let x = path_x(z) + side * (11.0 + 3.2 * size + params.stray_reach * pow(hash(vec2f(j, 12.2)), 1.5));
  // In clusters and gaps along each side, not evenly.
  let cluster = smoothstep(-0.3, 0.3, sin(i * 0.23 + side * 2.1) * sin(i * 0.087 + side * 0.7 + 1.0));
  let here = f32(hash(vec2f(j, 15.3)) < params.strays * mix(0.15, 1.4, cluster));
  return vec4f(x, z, size, here);
}

fn stray(j: f32) -> Gate {
  var g: Gate;
  let place = stray_place(j);
  g.kind = select(kind_of(j, 10.0), 2.0, hash(vec2f(j, 17.7)) < 0.4);
  let b = hash(vec2f(j, 18.1));
  g.broken = select(select(select(0.0, 1.0, b < params.broken), 2.0, b < params.broken * 0.7), 3.0, b < params.broken * 0.4);
  g.size = place.z;
  g.z = place.y;
  g.x = place.x;
  let w = hash(vec2f(j, 19.4));
  if (w < params.sunk) {
    g.base = -mix(0.3, 0.65, hash(vec2f(j, 11.3))) * 5.6 * g.size;
  } else {
    g.base = -mix(0.2, 1.8, hash(vec2f(j, 11.3)));
  }
  // Turned any way; leaning, a few of them far over.
  g.lean = (hash(vec2f(j, 14.8)) - 0.5) * 0.6;
  if (hash(vec2f(j, 14.1)) < 0.15) {
    g.lean = sign(g.lean) * mix(0.4, 0.7, hash(vec2f(j, 14.3)));
  }
  g.turn = (hash(vec2f(j, 26.1)) - 0.5) * 3.2;
  g.askew = (hash(vec2f(j, 27.3)) - 0.5) * 0.5;
  g.here = place.w;
  return g;
}

// The height of a torii's top.
fn gate_top(g: Gate) -> f32 {
  return g.base + 6.3 * g.size;
}

// Where the camera crosses the line's torii i: its centre across at the camera's height (a
// leaning torii's opening shifts with the height).
fn gate_x(i: f32) -> f32 {
  let g = gate(i);
  return g.x + (params.through - g.base) * tan(g.lean);
}

// The line through the torii at z: from one centre to the next, eased.
fn way_x(z: f32) -> f32 {
  let f = gate_pos(z);
  let i = floor(f);
  return mix(gate_x(i), gate_x(i + 1.0), smoothstep(0.0, 1.0, f - i));
}

// A point on the screen as the camera sees it (in px.p units, rolled with the camera) and its
// depth; behind the camera the depth is below 0.3.
fn project(p: vec3f) -> vec3f {
  let rel = p - state.cam_pos;
  let z = dot(rel, state.cam_fwd);
  let q = vec2f(dot(rel, state.cam_right), dot(rel, state.cam_up)) / max(z, 1e-3) / (state.lens * 0.8);
  return vec3f(rv_rotate(q, -state.roll), z);
}
