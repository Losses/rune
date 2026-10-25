// What Growth keeps from frame to frame. The tower stands on the y axis; y is up, in metres.

struct State {
  // The camera: where it is, the point it looks at, and its roll.
  cam_pos: vec3f,
  roll: f32,
  look: vec3f,
  lens: f32,
  // The tower's tip as the camera chases it, and where it has grown to.
  top: f32,
  top_to: f32,
  since_beat: f32,
  // How fast the music sounds (smoothed), its energy (smoothed twice) and the kick.
  feel: f32,
  energy: f32,
  ens: f32,
  kick: f32,
  // The view: its height below the tip, distance from the axis,
  // its roll, how fast it circles; now and where each is gliding to.
  below: f32,
  dist: f32,
  tilt: f32,
  below_to: f32,
  dist_to: f32,
  tilt_to: f32,
  azimuth: f32,
  spin: f32,
  spin_to: f32,
  last_view: f32,
  // The tip before the last beat's growth (what grew since is still flying in), and the flash of
  // an accent.
  top_prev: f32,
  flash: f32,
  last_accent: f32,
  last_energy: f32,
  // Growth per beat in layers, and the view chosen last.
  per_beat: f32,
  view: f32,
  // The tip as the camera follows it (smoothly), and how far below it the camera aims.
  cam_top: f32,
  aim: f32,
  aim_to: f32,
  // A beat's glitch: the picture inverted to blue and torn into bands, for a moment.
  glitch: f32,
  // The colours: fading from one theme to the next, the one waiting, and when it last changed.
  theme_from: f32,
  theme_to: f32,
  theme_mix: f32,
  theme_next: f32,
  last_theme: f32,
  // The colours now: the last theme fading into the next, in the player's colour if it follows
  // it (see update).
  theme: Theme,
}

// The grid the plates sit on: one plate at most in each cell.
const CELL: vec3f = vec3f(1.6, 0.9, 1.6);

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

// The white light's key light, from high on one side.
const SUN: vec3f = vec3f(0.42, 0.83, 0.36);

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

// The themes. The white light, as the video: 0 ivory (warm white and grey), 1 gold (white and
// amber, as it flares), 2 midnight (navy and steel blue, as its inverted frame), 3 rose, 4 ice.
// Concrete, one material under three lights: 5 a low warm sun from the side, 6 a high white sun,
// 7 the blue hour, the sun gone and the rod lighting the core. The light from below is what the
// sunlit ground throws back: the camera looks up, at the plates' undersides.
fn theme_of(m: f32) -> Theme {
  var t: Theme;
  if (m < 0.5) {
    t = Theme(vec3f(1.15, 1.13, 1.07), vec3f(0.97, 0.95, 0.94), vec3f(1.0, 0.95, 0.8),
      vec3f(0.78, 0.76, 0.71), vec3f(0.42, 0.43, 0.45), vec3f(0.12, 0.13, 0.15), vec3f(0.86, 0.78, 0.52),
      vec3f(1.7, 1.63, 1.5), vec3f(1.0, 0.98, 0.94), vec3f(0.3, 0.33, 0.4),
      vec3f(1.3, 1.27, 1.15), vec3f(0.12, 0.12, 0.13), SUN);
  } else if (m < 1.5) {
    t = Theme(vec3f(1.25, 1.2, 1.0), vec3f(1.05, 0.98, 0.82), vec3f(1.0, 0.85, 0.45),
      vec3f(0.92, 0.76, 0.38), vec3f(0.62, 0.47, 0.2), vec3f(0.2, 0.15, 0.08), vec3f(1.0, 0.95, 0.85),
      vec3f(1.9, 1.7, 1.2), vec3f(1.05, 0.98, 0.85), vec3f(0.4, 0.33, 0.25),
      vec3f(1.4, 1.25, 0.9), vec3f(0.25, 0.18, 0.08), SUN);
  } else if (m < 2.5) {
    t = Theme(vec3f(0.025, 0.045, 0.11), vec3f(0.008, 0.014, 0.035), vec3f(0.08, 0.13, 0.3),
      vec3f(0.55, 0.65, 0.8), vec3f(0.3, 0.38, 0.52), vec3f(0.04, 0.06, 0.12), vec3f(0.8, 0.9, 1.0),
      vec3f(0.6, 0.75, 1.1), vec3f(0.25, 0.35, 0.6), vec3f(0.05, 0.07, 0.14),
      vec3f(1.2, 1.5, 2.2), vec3f(0.5, 0.65, 0.9), SUN);
  } else if (m < 3.5) {
    t = Theme(vec3f(1.15, 1.02, 1.02), vec3f(0.95, 0.82, 0.84), vec3f(1.0, 0.75, 0.7),
      vec3f(0.85, 0.42, 0.42), vec3f(0.55, 0.22, 0.25), vec3f(0.15, 0.04, 0.06), vec3f(0.95, 0.88, 0.82),
      vec3f(1.7, 1.5, 1.45), vec3f(1.0, 0.92, 0.92), vec3f(0.35, 0.22, 0.26),
      vec3f(1.4, 1.1, 1.1), vec3f(0.2, 0.05, 0.07), SUN);
  } else if (m < 4.5) {
    t = Theme(vec3f(0.85, 1.08, 1.2), vec3f(0.62, 0.86, 0.95), vec3f(0.55, 0.85, 1.0),
      vec3f(0.55, 0.82, 0.92), vec3f(0.25, 0.5, 0.62), vec3f(0.02, 0.12, 0.18), vec3f(0.92, 0.98, 1.0),
      vec3f(1.5, 1.65, 1.75), vec3f(0.95, 1.0, 1.02), vec3f(0.22, 0.35, 0.42),
      vec3f(1.1, 1.35, 1.5), vec3f(0.05, 0.12, 0.16), SUN);
  } else if (m < 5.5) {
    // Concrete greys differ only a little: the light does the work.
    t = Theme(vec3f(0.05, 0.07, 0.11), vec3f(0.2, 0.17, 0.15), vec3f(0.9, 0.55, 0.28),
      vec3f(0.42, 0.41, 0.39), vec3f(0.36, 0.355, 0.345), vec3f(0.26, 0.255, 0.25), vec3f(0.46, 0.44, 0.41),
      vec3f(3.4, 2.7, 1.9), vec3f(0.1, 0.13, 0.19), vec3f(0.16, 0.12, 0.09),
      vec3f(2.0, 1.6, 1.1), vec3f(0.05, 0.045, 0.04), normalize(vec3f(0.78, 0.3, 0.45)));
  } else if (m < 6.5) {
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
