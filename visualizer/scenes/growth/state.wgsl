// What Growth keeps from frame to frame. The tower stands on the y axis; y is up, in metres.

struct State {
  // The camera: where it is, the point it looks at, and its roll.
  cam_pos: vec3f,
  roll: f32,
  look: vec3f,
  lens: f32,
  // The view's own field of view (1 is the usual, less is a longer lens), how far below the tip
  // the tower is drawn at least (more for a view from above), and the distance in focus.
  fov: f32,
  deep: f32,
  focus: f32,
  // The tower's tip as the camera chases it, and where it has grown to.
  top: f32,
  top_to: f32,
  since_beat: f32,
  // The kick.
  kick: f32,
  // The view: its height below the tip, distance from the axis,
  // its roll, how fast it circles; now and where each is gliding to.
  below: f32,
  dist: f32,
  tilt: f32,
  azimuth: f32,
  spin: f32,
  // How many cuts of the director the view has followed, and the view taken (its shot).
  cuts: u32,
  view: u32,
  // The tip before the last beat's growth (what grew since is still flying in), and how bright
  // the rod is.
  top_prev: f32,
  rod: f32,
  // Growth per beat in layers.
  per_beat: f32,
  // The tip as the camera follows it (smoothly) and how fast it rises, and how far below it the
  // camera aims.
  cam_top: f32,
  cam_rise: f32,
  aim: f32,
  // How full each layer is (the `fill` when the tip grew past its middle), four layers to an
  // element, the layer's index going round `FILL_LAYERS`.
  fills: array<vec4f, 64>,
  // The colours now: the last look fading into the next (`look`, the runtime's), in the player's
  // colour if it follows it (see update).
  theme: Theme,
}

// The grid the plates sit on: one plate at most in each cell.
const CELL: vec3f = vec3f(1.6, 0.9, 1.6);

// How many layers keep their own fullness (`fills`): far more than are ever in view.
const FILL_LAYERS: i32 = 256;

// Where layer y (its index) keeps its fullness in `fills`.
fn fill_slot(y: f32) -> u32 {
  return u32(((i32(y) % FILL_LAYERS) + FILL_LAYERS) % FILL_LAYERS);
}

// How full layer y is, 0 to 1, as the music was when it grew.
fn layer_fill(y: f32) -> f32 {
  let i = fill_slot(y);
  return state.fills[i / 4u][i % 4u];
}

// How many of its plates a layer lays, against the plain density, by how full it is: a quiet
// passage a few, a loud one all.
fn fill_count(fill: f32) -> f32 {
  return mix(0.15, 1.3, fill);
}

// How wide a layer is, against the tower's girth, from an empty layer to a full one: a quiet
// passage a thin column, a loud one wide.
const FILL_GIRTH: vec2f = vec2f(0.55, 1.35);

fn hash(p: vec2f) -> f32 {
  var p3 = fract(vec3f(p.x, p.y, p.x) * 0.1031);
  p3 += dot(p3, p3.yzx + 33.33);
  return fract((p3.x + p3.y) * p3.z);
}

fn hash3(p: vec3f) -> f32 {
  var p3 = fract(p * 0.1031);
  p3 += dot(p3, p3.zyx + 31.32);
  return fract((p3.x + p3.y) * p3.z);
}

// The colours of a theme: the haze above and below and the light's glow in it; plates pale, mid,
// dark and the few odd ones; the key light, and the sky light from above and below; the rod and
// the dust.
struct Theme {
  haze_up: vec3f,
  haze_down: vec3f,
  glow: vec3f,
  pale: vec3f,
  mid: vec3f,
  dark: vec3f,
  odd: vec3f,
  key: vec3f,
  sky_up: vec3f,
  sky_down: vec3f,
  rod: vec3f,
  dust: vec3f,
  // Towards the key light.
  sun: vec3f,
}

// The looks (the `[[look]]`s of scene.toml, in order): one material, concrete, under three lights: 0 a low warm sun from the side, 1 a
// high white sun, 2 the blue hour, the sun gone and the rod lighting the core. The light from
// below is what the sunlit ground throws back: the camera looks up, at the plates' undersides.
fn theme_of(m: f32) -> Theme {
  var t: Theme;
  if (m < 0.5) {
    // Concrete greys differ only a little: the light does the work.
    t = Theme(vec3f(0.05, 0.07, 0.11), vec3f(0.2, 0.17, 0.15), vec3f(0.9, 0.55, 0.28),
      vec3f(0.42, 0.41, 0.39), vec3f(0.36, 0.355, 0.345), vec3f(0.26, 0.255, 0.25), vec3f(0.46, 0.44, 0.41),
      vec3f(3.4, 2.7, 1.9), vec3f(0.1, 0.13, 0.19), vec3f(0.16, 0.12, 0.09),
      vec3f(2.0, 1.6, 1.1), vec3f(0.05, 0.045, 0.04), normalize(vec3f(0.78, 0.3, 0.45)));
  } else if (m < 1.5) {
    t = Theme(vec3f(0.1, 0.14, 0.22), vec3f(0.32, 0.35, 0.39), vec3f(0.5, 0.5, 0.45),
      vec3f(0.42, 0.41, 0.39), vec3f(0.36, 0.355, 0.345), vec3f(0.26, 0.255, 0.25), vec3f(0.46, 0.44, 0.41),
      vec3f(3.0, 2.95, 2.8), vec3f(0.17, 0.19, 0.23), vec3f(0.17, 0.16, 0.15),
      vec3f(2.0, 1.95, 1.85), vec3f(0.06, 0.06, 0.06), normalize(vec3f(0.3, 0.9, 0.3)));
  } else {
    t = Theme(vec3f(0.015, 0.025, 0.06), vec3f(0.09, 0.1, 0.17), vec3f(0.08, 0.08, 0.15),
      vec3f(0.42, 0.41, 0.39), vec3f(0.36, 0.355, 0.345), vec3f(0.26, 0.255, 0.25), vec3f(0.46, 0.44, 0.41),
      vec3f(0.22, 0.28, 0.45), vec3f(0.11, 0.15, 0.27), vec3f(0.02, 0.025, 0.04),
      vec3f(3.0, 2.0, 1.1), vec3f(0.02, 0.02, 0.03), normalize(vec3f(-0.6, 0.12, 0.5)));
  }
  return t;
}
