// What Armillary keeps from frame to frame: the music; the sphere floating in space and its
// rings, each moved as a body hung in its place is (pushed, swinging out and back or going round
// a whole turn, slowed by friction, nudged a moment later by another's push); the core of light, breathing with the music; the
// shot; and the shapes, big polygons drawn in fine lines that appear round it a batch at a time
// as accents light them, projected and sorted into screen tiles so a pixel looks only at
// the few that could touch it.

const RINGS: u32 = 5u;
const SHAPES: u32 = 96u;
// The shapes come in batches, each lit at once; so many batches, so many shapes in each.
const BATCHES: u32 = 4u;
const IN_BATCH: u32 = 24u;
// The cut gem's facets (core look 3).
const FACETS: u32 = 92u;
// The screen's tiles, and how many shapes a tile lists at most.
const TX: u32 = 32u;
const TY: u32 = 18u;
const TILES: u32 = 576u;
const LIST: u32 = 24u;

struct State {
  // The kick (1 on it, falling away), the loudness now (smoothed), and the live levels of the
  // bass, mids and treble (quick up, slower down).
  kick: f32,
  moment: f32,
  levels: vec3f,
  // Bars, phrases and sections counted.
  bars: f32,
  phrases: f32,
  sections: f32,
  // Accents, which light the rings drawn as lines or the shapes: which beat of the bar this is,
  // how many times something has lit, which of the two is lighting now (0 the rings, 1 the
  // shapes) and how far the picture has gone over to it (eased).
  beat_in_bar: f32,
  lits: f32,
  layer: f32,
  layer_mix: f32,
  // The shot and its side, and when it last changed.
  view: f32,
  side: f32,
  last_view: f32,
  // What keeps turning on its own: the tempo (smoothed), the ecliptic's slow precession (faster
  // as the music is louder), how far the camera has moved in this shot, and the core's flowing
  // surface and its turning rays.
  tempo: f32,
  spin: f32,
  drift_cam: f32,
  core_flow: f32,
  core_rays: f32,
  planet_turn: f32,
  // The core's size now (it swells with the music and on the kick, sprung), and its speed.
  core_size: f32,
  core_vel: f32,
  // How the gem turns, as a free ring does: where it turns from (its upright and a line across),
  // the axis it turns about, and the turn (angle, speed, the push behind it).
  gem_rest_a: vec4f,
  gem_rest_b: vec4f,
  gem_about: vec4f,
  gem_roll: vec4f,
  // How many facets the gem has (eighty triangles when it is drawn in lines).
  gem_count: u32,
  // Where the sphere's middle is now (the core of light, the centre of every ring): floating.
  centre: vec3f,
  // The camera.
  cam_pos: vec3f,
  cam_fwd: vec3f,
  cam_right: vec3f,
  cam_up: vec3f,
  focal: f32,
  // The rings: the rows of each one's turn from the world into its own frame (z its axis), their
  // w its outer radius, its width and half its thickness; and its look (x how its ticks are cut).
  ring_x: array<vec4f, 5>,
  ring_y: array<vec4f, 5>,
  ring_z: array<vec4f, 5>,
  ring_look: array<vec4f, 5>,
  // How each ring moves: where it rests (its axis and a line in its plane); the turn it is making
  // (angle, speed, the push behind it) about an axis; a push waiting to land (how much, how soon,
  // whether it is a whole turn, and what picks that turn's axis).
  ring_rest_a: array<vec4f, 5>,
  ring_rest_b: array<vec4f, 5>,
  ring_turn: array<vec4f, 5>,
  ring_about: array<vec4f, 5>,
  ring_wait: array<vec4f, 5>,
  // How lit each ring is (x, 1 at an accent's full strength, fading).
  ring_lit: array<vec4f, 5>,
  // Drawn in marks (look 3), the dots an accent switches on round each ring: where they start (x,
  // in turns), how far either way they have come on (y, turns), how bright they are (z, fading
  // together) and how far either way they will come on (w).
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
  // The gem's facets, turned with it: each one's outward normal (in the world).
  gem: array<vec4f, 92>,
  // The colours, all from the one theme colour (see update): the palette, a tint for each ring
  // and for each shape (each a little its own).
  palette: array<vec4f, 16>,
  ring_tint: array<vec4f, 5>,
  shape_tint: array<vec4f, 96>,
  tile_n: array<u32, 576>,
  tile: array<u32, 13824>,
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

fn screen_uv(p: vec2f, aspect: f32) -> vec2f {
  return vec2f(p.x / aspect + 0.5, 0.5 - p.y);
}
