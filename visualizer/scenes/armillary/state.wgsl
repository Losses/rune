// What Armillary keeps from frame to frame: the sphere hung in space and its rings, each
// moved as a body hung in its place is (pushed, swinging out and back or going round a whole turn,
// slowed by friction, nudged a moment later by another's push); the planet at its heart, swelling
// with the music; the shot; and the shapes, big polygons drawn in fine lines that appear round it
// a batch at a time as accents light them, projected and listed (those on the screen) so a pixel
// can skip at a glance the ones that cannot touch it.

// The sphere's middle: the planet, and the centre of every ring. It stays where it is (it does not
// float).
const CENTRE: vec3f = vec3f(0.0);

const RINGS: u32 = 5u;
const SHAPES: u32 = 96u;
// The shapes come in batches, each lit at once; so many batches, so many shapes in each.
const BATCHES: u32 = 4u;
const IN_BATCH: u32 = 24u;

struct State {
  // The beats the scene counts (see update): how many, and the seconds since the last. And the
  // scene's own time for everything that moves of itself (the shapes' turn in their planes): it
  // advances by `dt` times how much is heard (`input.audible`), so it slows with a fade and stands
  // still in silence. (`frame.time` stays only in the stars' twinkle, which is light, not motion.)
  count: f32,
  beat_age: f32,
  phase: f32,
  // How far the picture is in the theme colour look (0 the grey look, 1 the colour one).
  colour: f32,
  // Bars, phrases and sections (strong cues) counted.
  bars: f32,
  phrases: f32,
  sections: f32,
  // Accents, which light the rings' dots or the shapes: which counted beat of the bar this is,
  // how many times something has lit, which of the two is lighting now (0 the rings, 1 the
  // shapes) and how far the picture has gone over to it (eased).
  beat_in_bar: f32,
  lits: f32,
  layer: f32,
  layer_mix: f32,
  // The shot, its side, and how many of the director's cuts have been taken up.
  view: f32,
  cuts_seen: f32,
  side: f32,
  // The kicks between the beats (see update): the kick's envelope a frame ago (to tell a jump up),
  // and the batch of shapes lit last (-1 none).
  kick_prev: f32,
  last_batch: f32,
  // The burst's cut (see `choose_burst` in update): what it frames, decided at the cut and kept for
  // it: the ring or shape it looks at and the angle on the ring, how far the camera is, the
  // direction it looks from, an offset of what it aims at, and the directions the burst has
  // looked from so far (summed, so the last cut looks from another side).
  burst_a: f32,
  burst_phi: f32,
  burst_dist: f32,
  burst_dir: vec3f,
  burst_off: vec3f,
  burst_seen: vec3f,
  // 1 while the shape the burst looks at is held lit.
  burst_hold: f32,
  // The planet's size now (it swells with the music and on each beat, sprung), and its speed.
  core_size: f32,
  core_vel: f32,
  // The camera.
  cam_pos: vec3f,
  cam_fwd: vec3f,
  cam_right: vec3f,
  cam_up: vec3f,
  focal: f32,
  // The rings: the rows of each one's turn from the world into its own frame (z its axis), the w
  // of the first its outer radius and of the second its width.
  ring_x: array<vec4f, 5>,
  ring_y: array<vec4f, 5>,
  ring_z: array<vec4f, 5>,
  // How each ring moves: where it rests (its axis and a line in its plane); the turn it is making
  // (angle, speed, the push behind it) about an axis; a push waiting to land (how much, how soon,
  // whether it is a whole turn, and what picks that turn's axis).
  ring_rest_a: array<vec4f, 5>,
  ring_rest_b: array<vec4f, 5>,
  ring_turn: array<vec4f, 5>,
  ring_about: array<vec4f, 5>,
  ring_wait: array<vec4f, 5>,
  // The dots an accent lights round each ring, all at once: where they
  // start (x, in turns), how bright they are now (y, rising to z), how bright the accent left
  // them (z, fading) and how far either way from the start they are lit (w, turns).
  ring_on: array<vec4f, 5>,
  // Which ring is drawn as a line of the spectrum now, and how far each one is (x, eased).
  spec_ring: f32,
  ring_spec: array<vec4f, 5>,
  // Each shape: where it is and how big (w, 0 if it is not there); its plane's normal and how
  // many sides it has (w); a line in its plane (where its first corner points); its look (y how
  // lit it is, fading; z how many times it has been lit, which picks where and what it is); and
  // where it is on the screen (centre, how far it reaches, depth; w < 0 if it is not drawn).
  shape_pos: array<vec4f, 96>,
  shape_n: array<vec4f, 96>,
  shape_u: array<vec4f, 96>,
  shape_look: array<vec4f, 96>,
  shape_scr: array<vec4f, 96>,
  // Each batch of shapes, set as it lights: the axis of the band they ring the sphere round
  // (facing the camera then, so the batch rings the sphere as it is seen); how many there are
  // (x) and how big they are drawn for the camera's distance then (y).
  batch_axis: array<vec4f, 4>,
  batch_look: array<vec4f, 4>,
  // The colours, all from the one theme colour (see update): the palette, a tint for each ring
  // and for each shape (each a little its own).
  palette: array<vec4f, 8>,
  ring_tint: array<vec4f, 5>,
  shape_tint: array<vec4f, 96>,
  // The shapes that are drawn this frame, in a short list: how many, and for each its centre on
  // the screen, how far it reaches and which shape it is (as a number, in w). The list is made
  // with one write a shape, whatever size a shape is on the screen; sorting them into screen tiles
  // took a write for every tile a big shape covers, and that single thread was the frame's
  // slowest part, uneven as it was with how many shapes were lit.
  hit_n: u32,
  hit: array<vec4f, 96>,
}

fn hash(p: vec2f) -> f32 {
  var p3 = fract(vec3f(p.x, p.y, p.x) * 0.1031);
  p3 += dot(p3, p3.yzx + 33.33);
  return fract((p3.x + p3.y) * p3.z);
}

// v turned by angle t about the unit axis a.
fn turn(v: vec3f, a: vec3f, t: f32) -> vec3f {
  return v * cos(t) + cross(a, v) * sin(t) + a * dot(a, v) * (1.0 - cos(t));
}

// The polar axis: tipped from upright to the latitude.
fn pole() -> vec3f {
  let lat = radians(params.latitude);
  return vec3f(0.0, sin(lat), cos(lat));
}

// The planet's radius: it swells with the music and on each beat (view.wgsl draws it, and the
// burst frames it).
fn planet_radius() -> f32 {
  return params.planet * (0.82 + 0.35 * clamp(state.core_size - 1.0, 0.0, 0.8));
}

fn screen_uv(p: vec2f, aspect: f32) -> vec2f {
  return vec2f(p.x / aspect + 0.5, 0.5 - p.y);
}
