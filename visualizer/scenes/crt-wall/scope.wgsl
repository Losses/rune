// The oscilloscopes' picture, from the waveform that is playing, into a small buffer that keeps
// the last frame: the beam's trace fades over a tenth of a second, as a phosphor's glow does.
// The left half is the stereo picture (left against right, turned so that mono stands upright);
// the right half the wave against time, from a rising zero crossing so it stands still. Each is
// drawn in -1..1 both ways; a screen stretches it to its shape.

const POINTS: i32 = 96;

fn shade(px: Pixel) -> vec4f {
  let right = px.uv.x >= 0.5;
  let a = vec2f(fract(px.uv.x * 2.0) * 2.0 - 1.0, 1.0 - px.uv.y * 2.0);
  // The beam's width: about two texels.
  let w = 4.0 / px.size.y;
  var d = 1e9;
  var last = vec2f(0.0);
  for (var i = 0; i < POINTS; i++) {
    var p: vec2f;
    if (right) {
      // A window half the waveform long, from the trigger.
      let x = f32(i) / f32(POINTS - 1);
      let s = rv_waveform(state.trig + x * 0.5);
      p = vec2f(x * 2.0 - 1.0, 0.5 * (s.x + s.y) * state.gain);
    } else {
      let s = rv_waveform(f32(i) / f32(POINTS - 1)) * state.gain;
      p = vec2f(s.x - s.y, s.x + s.y) * 0.62;
    }
    if (i > 0) {
      d = min(d, rv_sd_segment(a, last, p));
    }
    last = p;
  }
  let beam = exp(-d * d / (w * w));
  let before = textureSampleLevel(prev_scope_buf, linear_clamp, px.uv, 0.0).r;
  let keep = exp(-frame.dt / max(params.persist, 0.01));
  return vec4f(max(beam, before * keep), 0.0, 0.0, 1.0);
}
