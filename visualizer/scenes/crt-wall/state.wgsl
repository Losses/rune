// What CRT Wall keeps from frame to frame. The wall's face is the plane z = 0, the sets stand out
// of it towards the camera (negative z), y is up, in metres.

struct State {
  // The camera: where it is, which way it turns (yaw) and tilts (pitch), its lens (half the
  // picture's height at one metre), and the view it is in.
  cam: vec3f,
  yaw: f32,
  pitch: f32,
  lens: f32,
  view: f32,
  last_view: f32,
  // Which way it travels along the wall (+1 or -1), how fast now and the speed it eases to.
  dir: f32,
  vel: f32,
  vtgt: f32,
  // How fast the music sounds (smoothed), the energy (smoothed twice) and the kick.
  feel: f32,
  energy: f32,
  ens: f32,
  kick: f32,
  // The beat clock in beats (its whole part the beats so far), seconds per beat, and whether a
  // beat is followed.
  clock: f32,
  beat_len: f32,
  sure: f32,
  // The scopes: where in the waveform the time trace starts (a rising zero crossing, so it stands
  // still) and the gain that keeps the trace a steady size.
  trig: f32,
  gain: f32,
  // The share of screens switched off in this section, and the section's number (how the
  // channels group).
  dark: f32,
  section: f32,
  last_energy: f32,
  // The phosphor's colour and the key light's, worked out once a frame.
  phosphor: vec3f,
  key: vec3f,
}

fn hash(p: vec2f) -> f32 {
  var p3 = fract(vec3f(p.x, p.y, p.x) * 0.1031);
  p3 += dot(p3, p3.yzx + 33.33);
  return fract((p3.x + p3.y) * p3.z);
}

// Towards the key light: high on the left, in front of the wall.
const KEY_DIR: vec3f = vec3f(-0.42, 0.74, -0.53);
