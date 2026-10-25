// What Paperclip keeps from frame to frame. The floor is y = 0, y is up; the lead stands at the
// origin facing the camera's side (-z), and the chorus stands in rows behind it (+z). Lengths are
// in clip heights.

// How many dancers are posed each frame: the lead, then the chorus in eight groups, each a step
// further behind the beat (the wave spreads out from the lead).
const GROUPS: u32 = 9u;
// Points of a posed clip: the wire's centre line, then the eyes (centre, look in w) and the brows
// (two ends each).
const WIRE: u32 = 26u;
const POINTS: u32 = 32u;

struct State {
  // The camera: where it is, where it looks and the lens; where it is round the lead (bearing, 0
  // in front and positive to the right, then distance out over the floor and height) and where it
  // looks before it follows the lead's hops; where a glide to another view started, how far it
  // has got and how long it takes; the view, which side of the lead it films from, how far through
  // the view's own slow move it is, when it was chosen and for which section; the circling view's
  // bearing; the lead's hop, smoothed.
  cam: vec3f,
  lens: f32,
  aim: vec3f,
  glide: f32,
  pol: vec3f,
  glide_len: f32,
  aim_base: vec3f,
  view: f32,
  pol_from: vec3f,
  lens_from: f32,
  aim_from: vec3f,
  last_view: f32,
  side: f32,
  shot: f32,
  planned: f32,
  ring: f32,
  lift: f32,
  // The beat clock in beats, seconds per beat, beats per bar, where bars fall on it, and whether
  // a beat is followed.
  clock: f32,
  beat_len: f32,
  per_bar: f32,
  bar_shift: f32,
  sure: f32,
  // How hard they dance (the section's energy, smoothed), the brows' lift on an accent and when
  // the last one was, the section (it picks the moves).
  level: f32,
  brow: f32,
  last_accent: f32,
  section: f32,
  // Which way the dancers face (radians, 0 towards -z): where a turn started, where it ends and the
  // bar it started on.
  face_from: f32,
  face_to: f32,
  face_bar: f32,
  energy: f32,
  last_energy: f32,
  // The face of the section or phrase (see face_at), the one before it and the bar it changed on
  // (it spreads through the chorus in the wave), and the phrase it was chosen for; the surprise
  // of a section that lifts the energy.
  mood: f32,
  mood_prev: f32,
  mood_bar: f32,
  mood_phrase: f32,
  surprise: f32,
  // The paper's colour.
  paper: vec3f,
  // Each group's clip posed: its points (the first six also carry, in w, which way its head faces
  // and which way is up on it), and the sphere that holds it (centre, radius).
  pts: array<vec4f, 288>,
  bound: array<vec4f, 9>,
  // Each group's face as shown: the face, when it was put on, the face before and whether it is
  // one the dance requires (see show_face).
  faces: array<vec4f, 9>,
}

fn hash(p: vec2f) -> f32 {
  var p3 = fract(vec3f(p.x, p.y, p.x) * 0.1031);
  p3 += dot(p3, p3.yzx + 33.33);
  return fract((p3.x + p3.y) * p3.z);
}

// Towards the key light: high on the left, in front.
const LIGHT: vec3f = vec3f(-0.45, 0.8, -0.4);
// The wire's radius.
const WIRE_R: f32 = 0.03;
const EYE_R: f32 = 0.082;
const BROW_R: f32 = 0.012;
// How far the face on the eyes turns from the head's front towards the camera (radians).
const GLANCE: f32 = 0.7;
// The chorus's grid on the floor, and the first row behind the lead.
const CELL: f32 = 1.3;
const FIRST_ROW: f32 = 2.0;

// The wire's centre line, a gem clip one high, from the inner end: up, the inner top bend, down,
// the bottom bend, up the outer right, the big top bend, down the outer left to the other end.
const PATH: array<vec2f, 26> = array<vec2f, 26>(
  vec2f(0.0700, 0.2700),
  vec2f(0.0700, 0.4900),
  vec2f(0.0700, 0.7100),
  vec2f(0.0495, 0.7595),
  vec2f(0.0000, 0.7800),
  vec2f(-0.0495, 0.7595),
  vec2f(-0.0700, 0.7100),
  vec2f(-0.0700, 0.5100),
  vec2f(-0.0700, 0.3100),
  vec2f(-0.0700, 0.1100),
  vec2f(-0.0490, 0.0453),
  vec2f(0.0060, 0.0054),
  vec2f(0.0740, 0.0054),
  vec2f(0.1290, 0.0453),
  vec2f(0.1500, 0.1100),
  vec2f(0.1500, 0.3333),
  vec2f(0.1500, 0.5567),
  vec2f(0.1500, 0.7800),
  vec2f(0.1299, 0.8550),
  vec2f(0.0750, 0.9099),
  vec2f(0.0000, 0.9300),
  vec2f(-0.0750, 0.9099),
  vec2f(-0.1299, 0.8550),
  vec2f(-0.1500, 0.7800),
  vec2f(-0.1500, 0.5150),
  vec2f(-0.1500, 0.2500));

// The chorus group of the dancer in floor cell c: further from the lead, later.
fn group_of(c: vec2f) -> u32 {
  let d = clamp(length((c + 0.5) * CELL) * params.wave, 0.0, 0.999);
  return 1u + u32(d * 8.0);
}
