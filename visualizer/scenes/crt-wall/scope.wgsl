// The oscilloscopes' picture, from the waveform that is playing, into a small buffer that keeps
// the last frame: the beam's trace fades over a tenth of a second, as a phosphor's glow does.
// The left half is the stereo picture (left against right, turned so that mono stands upright);
// the right half the wave against time, from a rising zero crossing so it stands still. Each is
// drawn in -1..1 both ways; a screen stretches it to its shape.

// A point of the stereo picture, from the plan.
fn scope_point_in_plan(i: i32) -> vec2f {
  let t = textureLoad(plan, vec2i((i % 48) / 2, i32(PLAN_SCOPE) + i / 48), 0);
  return select(t.xy, t.zw, i % 2 == 1);
}

fn shade(px: Pixel) -> vec4f {
  let right = px.uv.x >= 0.5;
  let a = vec2f(fract(px.uv.x * 2.0) * 2.0 - 1.0, 1.0 - px.uv.y * 2.0);
  // The beam's width: about two texels.
  let w = 4.0 / px.size.y;
  var d = 1e9;
  var last = vec2f(0.0);
  // Only what comes within six beam widths can light this texel (further, the beam is below
  // what the buffer holds). The time trace runs left to right, a point every 2 / (POINTS - 1):
  // the points around this texel's x.
  var first = 0;
  var end = POINTS;
  if (right) {
    let reach = 6.0 * w;
    let per = 0.5 * f32(POINTS - 1);
    first = clamp(i32(floor((a.x - reach + 1.0) * per)), 0, POINTS - 1);
    end = clamp(i32(ceil((a.x + reach + 1.0) * per)) + 1, 1, POINTS);
  }
  if (!right && all(textureDimensions(plan) >= vec2u(PLAN_WIDTH, PLAN_HEIGHT))) {
    // The stereo picture from the plan: its points, and the box around each run of eight
    // segments, so that only the runs within reach are looked at.
    let reach = 6.0 * w;
    for (var r = 0; r < SCOPE_RUNS; r++) {
      let bounds = textureLoad(plan, vec2i(r, i32(PLAN_SCOPE) + 2), 0);
      if (all(a >= bounds.xy - reach) && all(a <= bounds.zw + reach)) {
        var p0 = scope_point_in_plan(r * 8);
        for (var k = 1; k <= 8; k++) {
          let i = r * 8 + k;
          if (i >= POINTS) {
            break;
          }
          let p1 = scope_point_in_plan(i);
          d = min(d, rv_sd_segment(a, p0, p1));
          p0 = p1;
        }
      }
    }
    first = POINTS;
  }
  for (var i = first; i < end; i++) {
    var p: vec2f;
    if (right) {
      // A window half the waveform long, from the trigger.
      let x = f32(i) / f32(POINTS - 1);
      let s = rv_waveform(state.trig + x * 0.5);
      p = vec2f(x * 2.0 - 1.0, 0.5 * (s.x + s.y) * state.gain);
    } else {
      p = stereo_point(i);
    }
    if (i > first) {
      d = min(d, rv_sd_segment(a, last, p));
    }
    last = p;
  }
  let beam = exp(-d * d / (w * w));
  let before = textureSampleLevel(prev_scope_buf, linear_clamp, px.uv, 0.0).r;
  let keep = exp(-frame.dt / max(params.persist, 0.01));
  return vec4f(max(beam, before * keep), 0.0, 0.0, 1.0);
}
