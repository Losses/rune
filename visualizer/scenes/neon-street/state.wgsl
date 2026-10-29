// What Neon Street keeps from frame to frame. x runs across the street, y up, z along it.

struct State {
  // The camera: where it is, where it looks and its lens (1, less to zoom in).
  pos: vec3f,
  yaw: f32,
  pitch: f32,
  roll: f32,
  lens: f32,
  // The view chosen at the last cut: across, eye height, yaw and pitch, and which shot (the
  // director's index).
  lat: f32,
  eye: f32,
  yaw_base: f32,
  pitch_base: f32,
  view: f32,
  // Gliding: speed and where it heads.
  vel: f32,
  vtarget: f32,
  // The accents that shake the camera, and the shake itself.
  accent: f32,
  shake: f32,
  // When the music last had a hit that was heard (scene time), and the gap before it: the big
  // screen's ring starts from it.
  ring_time: f32,
  ring_gap: f32,
  // The lanterns' swing on their strings (an angle, and how fast it changes): pushed one way and
  // the other, dying away between; the count of pushes (for the side) and when the last was.
  sway: f32,
  sway_v: f32,
  sway_n: f32,
  sway_time: f32,
  // How far the dot-matrix boards and the big screens' bands have scrolled, in beats: a phase
  // that moves at the song's pace (`scroll_t` is the speed it heads for, in beats a second,
  // pushed up on a beat and easing off, as the camera's `vtarget`), and when it was last pushed.
  scroll: f32,
  scroll_v: f32,
  scroll_t: f32,
  scroll_time: f32,
  // The theme colour's palette (see update): black, deep, full and near white, all in its hue.
  theme_ramp: array<vec4f, 4>,
  // How many cuts the director had made when the view last changed.
  cuts: u32,
  // 1 at a cut, fading out: the trail is cleared across a cut.
  clear: f32,
  // The share of signs that are dark, how far the signs' patterns and the big screens' modes have
  // moved on (one for each hit that is heard, less in a fade), the colours of the district (a
  // hue, 0..1) and the traffic lights (0 red, 1 green).
  dark: f32,
  changes: f32,
  district: f32,
  signal: f32,
  // How far the picture is in the theme colour's look (0 the grey look, 1 the colour one), the
  // two looks blended as the runtime fades them.
  colour: f32,
  // The subject the burst's detail shot films (see hero_row): its kind (HERO_*), its row and side,
  // and 1 while it stands there (from the cut to it to the next cut), else 0.
  hero: vec4f,
  // The rows ahead of the camera, worked out once a frame (update's fill_rows) so the pixels
  // need not: row0 is the row behind the camera's, and row i of the tables is row0 + i. For each
  // row and side (i * 2 + 0 left, + 1 right): its sign (as sign_of); its wall, what stands
  // before the shop (as furniture), an awning (1) and a pole (1); and the building's first row,
  // rows, height and the kind of its upper floors. For each row the people walking in it now,
  // two of each stream (i * 6 + stream * 2 + j, as person(); height 0 for nobody).
  row0: f32,
  row_sign: array<vec4f, 56>,
  row_shop: array<vec4f, 56>,
  row_front: array<vec4f, 56>,
  row_people: array<vec4f, 168>,
}

// A hash of a point, 0..1 (Dave Hoskins' hash12): fractional inputs are fine.
fn h21(p: vec2f) -> f32 {
  var p3 = fract(vec3f(p.x, p.y, p.x) * 0.1031);
  p3 += dot(p3, p3.yzx + 33.33);
  return fract((p3.x + p3.y) * p3.z);
}

// ---- The street's layout ----
// What stands where along the street, here so that update can work out, once a frame, what the
// rows ahead of the camera hold (see the row tables in State and update's fill_rows). The city
// (city.wgsl) explains the grid.

const HALF: f32 = 3.6;
const UNIT: f32 = 4.5;
const COL: f32 = 9.0;
const STOREY: f32 = 3.2;
const BLOCK: f32 = 9.0;
const FAR: f32 = 180.0;
const ROAD: f32 = 2.9;
// How many rows ahead the things in the street are looked for.
const ROWS: u32 = 28u;

// The kinds of shop on the ground floor.
const KONBINI: f32 = 0.0;
const IZAKAYA: f32 = 1.0;
const GAMES: f32 = 2.0;
const BOUTIQUE: f32 = 3.0;
const DRUGSTORE: f32 = 4.0;
const CAFE: f32 = 5.0;
const BAR: f32 = 6.0;
const VENDING: f32 = 7.0;
const SHUTTER: f32 = 8.0;

fn crossing(n: f32) -> bool {
  return n - BLOCK * floor(n / BLOCK) < 0.5;
}

// ---- The burst's subject ----
// The burst's detail shot films one thing up close, and to be sure it is there it puts it there:
// at the cut update picks the kind from the shot's seed (the same section, the same subject), a
// row a little ahead and a side, and the row's generators give way to it until the next cut: its
// signs, a torii over the street and a pole go, and on its side what stands before the shop is
// the subject (or nothing, where the subject stands at the kerb). The street is as it was again
// after the cut away.
const HERO_TORII: f32 = 0.0;
const HERO_BIKE: f32 = 1.0;
const HERO_SIGNAL: f32 = 2.0;
const HERO_LAMP: f32 = 3.0;
const HERO_VENDING: f32 = 4.0;
const HERO_SHRINE: f32 = 5.0;
const HERO_KINDS: f32 = 6.0;

fn hero_row(n: f32) -> bool {
  return state.hero.w > 0.5 && n == state.hero.y;
}

// How busy the block of row n is, 0 quiet to 1 busy.
fn busy(n: f32) -> f32 {
  return h21(vec2f(floor(n / BLOCK), 77.7));
}

// Blocks with lanterns over the street; the others have poles and wires.
fn lantern_block(n: f32) -> bool {
  return h21(vec2f(floor(n / BLOCK), 12.1)) < 0.35;
}

// Some shop buildings take two rows; a building is named by its first row.
fn merged(k: f32, s: f32) -> bool {
  return h21(vec2f(k, s * 5.3 + 2.0)) < 0.4 && !crossing(2.0 * k) && !crossing(2.0 * k + 1.0);
}

fn first_row(n: f32, s: f32) -> f32 {
  let k = floor(n * 0.5);
  return select(n, 2.0 * k, merged(k, s));
}

fn rows_of(n: f32, s: f32) -> f32 {
  return select(1.0, 2.0, merged(floor(n * 0.5), s));
}

// How far shop building n on side s stands back from the street; most stand at its edge.
fn inset_of(n: f32, s: f32) -> f32 {
  let h = h21(vec2f(first_row(n, s), s * 2.1 + 21.1));
  return select(0.0, 0.5 + 1.6 * fract(h * 7.3), h < 0.4);
}

// Where the front of shop building n on side s is, across (always positive).
fn wall_x(n: f32, s: f32) -> f32 {
  return HALF + inset_of(n, s);
}

// What the upper floors of shop building n are: a giant screen, neon tubes, glass offices,
// flats with balconies, or windows with panels.
fn front_kind(n: f32, s: f32) -> f32 {
  return h21(vec2f(first_row(n, s), s) + 12.7);
}

// The height of the building in column i, row n; 0 for the street, cross streets and empty lots.
fn height_of(i: i32, n: f32) -> f32 {
  if (i == 0 || crossing(n)) {
    return 0.0;
  }
  let s = select(-1.0, 1.0, i > 0);
  if (abs(i) == 1) {
    let h = h21(vec2f(first_row(n, s), s * 3.7 + 11.0));
    // Mostly two to eight floors, now and then a tower.
    let floors = select(2.0 + floor(h * 7.5), 10.0 + floor(fract(h * 13.0) * 6.0), h > 0.88);
    return STOREY * floors + 3.0;
  }
  // Behind the shops the buildings are twice as long and taller.
  let h = h21(vec2f(floor(n * 0.5) * 1.3 + f32(i) * 17.1, 4.7));
  if (h < 0.06) {
    return 0.0;
  }
  return STOREY * (3.0 + floor(h * h * 16.0)) + 3.0;
}

// The shopping street's arch at the start of block n (its first row past the cross street), on
// most blocks: two posts and a board over the whole street with the street's name (city.wgsl).
fn arch_on(n: f32) -> bool {
  let b = floor(n / BLOCK);
  return n - b * BLOCK > 0.5 && n - b * BLOCK < 1.5 && h21(vec2f(b, 61.1)) < params.arches;
}

// Whether row n holds a torii over the street (a big gate spanning it, square on to the camera
// going down it) in the place of the signs hanging over it. Not at a cross street, at the
// shopping street's arch or where a pole stands.
fn street_torii(n: f32) -> bool {
  if (hero_row(n)) {
    return state.hero.x == HERO_TORII;
  }
  return h21(vec2f(n, 64.3)) < params.street_torii && !crossing(n) && !arch_on(n) && !pole_on(n);
}

// The sign building n on side s hangs out over the street, facing the camera: z of its face,
// width (0: none), bottom, top. Busy blocks have more and bigger signs; each block has one big
// sign on each side. Only `street_signs` of what the street once had are left, the big ones a
// little more often than the rest (2.5 times the share), and a row that holds a torii over the
// street has none.
fn sign_of(n: f32, s: f32) -> vec4f {
  if (street_torii(n) || hero_row(n)) {
    return vec4f(0.0);
  }
  let id = vec2f(n, s);
  let block = floor(n / BLOCK);
  let hb = height_of(i32(s), n);
  let hero = n - block * BLOCK == 1.0 + floor(h21(vec2f(block, s) + 5.5) * (BLOCK - 1.0));
  if (hero && hb > 9.0 && h21(id + 8.8) < min(2.5 * params.street_signs, 1.0)) {
    return vec4f(n * UNIT + 0.5 + 2.5 * h21(id + 3.9), 1.35, 3.2, hb - 0.5);
  }
  let b = busy(n);
  let y0 = 3.4 + 1.4 * h21(id + 1.3);
  let y1 = min(y0 + 3.0 + 7.0 * h21(id + 2.6) * mix(0.6, 1.0, b), hb - 0.4);
  let on = step(h21(id + 0.9), mix(0.4, 0.95, b) * params.street_signs) * step(1.6, y1 - y0) * f32(!crossing(n));
  return vec4f(n * UNIT + 0.4 + 3.4 * h21(id + 3.9), (0.6 + 0.45 * h21(id + 5.2)) * mix(0.9, 1.15, b) * on, y0, y1);
}

fn shop_kind(n: f32, s: f32) -> f32 {
  return floor(h21(vec2f(n, s) + 8.3) * 9.0);
}

// What stands on the pavement before shop (n, s): 0 nothing, 1 banners, 2 a standing board, 3 a
// row of bicycles, 4 a roadside shrine (where it stands, the shop has no awning), 5 vending
// machines, 6 the burst's bicycle. Nothing at a cross street; before shutters and vending
// machines only a shrine. On the burst subject's side of its row, the subject or nothing (a torii
// over the street leaves the pavement as it is).
fn furniture(n: f32, s: f32) -> f32 {
  if (crossing(n)) {
    return 0.0;
  }
  if (hero_row(n) && s == state.hero.z && state.hero.x != HERO_TORII) {
    let kind = state.hero.x;
    return select(select(select(0.0, 4.0, kind == HERO_SHRINE), 5.0, kind == HERO_VENDING), 6.0, kind == HERO_BIKE);
  }
  let id = vec2f(n, s);
  if (h21(id + 91.3) < params.shrines) {
    return 4.0;
  }
  let kind = shop_kind(n, s);
  if (kind == SHUTTER || kind == VENDING) {
    return 0.0;
  }
  // Standing boards are the commonest: a shopping street has one before most shops.
  let r = h21(id + 92.7) / max(params.street_things, 1e-3);
  return select(select(select(select(0.0, 5.0, r < 1.0), 3.0, r < 0.8), 2.0, r < 0.62), 1.0, r < 0.28);
}

// How many vending machines stand side by side before shop (n, s), where it has them: two or
// three (city.wgsl draws them).
fn vendings_of(n: f32, s: f32) -> f32 {
  return 2.0 + floor(h21(vec2f(n, s) + 97.1) * 2.0);
}

// Whether shop (n, s) has an awning: not over a shutter or a cross street, nor on the side of the
// burst's subject (a traffic light's or a lamp's pole would go through it).
fn awning_on(n: f32, s: f32) -> bool {
  let id = vec2f(n, s);
  let kind = shop_kind(n, s);
  let some = h21(id + 41.1) < 0.5;
  return !crossing(n) && !(hero_row(n) && s == state.hero.z) && furniture(n, s) != 4.0 && (kind == CAFE || (some && (kind == BOUTIQUE || kind == DRUGSTORE)));
}

// Concrete poles stand at the street's edge every other row, in blocks without lanterns.
fn pole_on(n: f32) -> bool {
  return !hero_row(n) && !lantern_block(n) && !crossing(n) && fract(n * 0.5) < 0.25;
}

// People walk the street in three streams, each its own way and pace: two up it, one down. In a
// stream, person m has a place a row long in a frame moving with the stream, so they walk on
// steadily the whole length of the street, never stepping back. How far stream k has moved, and
// its pace (signed).
fn stream(k: f32) -> vec2f {
  var pace = array<f32, 3>(1.25, -1.15, 0.95);
  let v = pace[u32(k)];
  return vec2f(v * frame.time, v);
}

// Person m of stream k: x, z, height and the phase of their stride; height 0 for nobody.
fn person(m: f32, k: f32) -> vec4f {
  let id = vec2f(m, k * 3.3);
  if (h21(id + 1.1) > params.people * 0.75) {
    return vec4f(0.0);
  }
  let way = stream(k);
  let z = (m + h21(id + 3.3)) * UNIT + way.x;
  let x = (h21(id + 4.4) * 2.0 - 1.0) * (ROAD - 0.3);
  let height = 1.55 + 0.3 * h21(id + 5.5);
  return vec4f(x, z, height, abs(way.x) * 2.6 + h21(id) * 6.28);
}

// The person of stream k walking in row n now, the j-th (0 or 1) of the two whose places may
// reach into it: their number m, or a number for nobody (whose person() is nobody or elsewhere).
fn person_in_row(n: f32, k: f32, j: f32) -> f32 {
  return floor((n * UNIT - stream(k).x) / UNIT) + j;
}
