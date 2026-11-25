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
  // has got and how long it takes; the view (the director's shot), which side of the lead it films
  // from, how far through the view's own slow move it is (`travel`), the cuts seen so far; the
  // dutch angle now and where the glide to this view's began; the circling view's bearing; the
  // lead's hop, smoothed.
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
  roll_from: f32,
  side: f32,
  travel: f32,
  roll: f32,
  ring: f32,
  lift: f32,
  cuts: u32,
  // The bar the last frame was in (a whole number), so that a new bar is seen once.
  bar_idx: f32,
  // How hard they dance (from the drive), the brows' lift on an impact or a lift of the energy,
  // the section (it picks the moves).
  level: f32,
  brow: f32,
  section: f32,
  // Which way the dancers face (radians, 0 towards -z): where a turn started, where it ends and the
  // bar it started on.
  face_from: f32,
  face_to: f32,
  face_bar: f32,
  // The face of the section or cue (see face_at), the one before it and the bar it changed on
  // (it spreads through the chorus in the wave); the surprise of a section that lifts the energy.
  mood: f32,
  mood_prev: f32,
  mood_bar: f32,
  surprise: f32,
  // How hard they dance now: the dance's strength, the energy of the moment and how much is heard
  // (`level` above is only the section's: it decides what comes, not how big).
  amp: f32,
  // The side the burst's second cut filmed from (its third films from the other).
  burst_side: f32,
  // What each of the last four bars does, decided once as the bar begins on the lead (slot
  // n % 4; see `decide_bar`): the bar number, its big move (0 none to 6 a backflip), the side
  // it goes to (-1 or 1), whether the bar is spent dizzy (1 or 0) and the amplitude it is danced
  // at. The chorus dances the same bar later in the wave and reads the same record, so nothing
  // changes in the middle of a bar, whatever the section, the cues or the level do. A bar with
  // no record has no move. Then the bar last decided, the bar of the last big move and how
  // many bars must pass before the next one.
  rec_bar: vec4f,
  rec_move: vec4f,
  rec_side: vec4f,
  rec_dizzy: vec4f,
  rec_amp: vec4f,
  gate_last: f32,
  move_bar: f32,
  move_gap: f32,
  // Without a followed beat, the hits the dance is on: the times (seconds) of the last eight, how
  // many there have been and when the last one counted (hits closer than half a beat are one).
  hit_t: array<vec4f, 2>,
  hit_n: u32,
  hit_last: f32,
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
