// The street with a little of the last frames left in it: motion blur that streaks the lights
// as the camera walks, none across a cut.

fn shade(px: Pixel) -> vec4f {
  let now = textureSampleLevel(inked, linear_clamp, px.uv, 0.0).rgb;
  // High up the camera flies fast: a long trail there would show every window twice.
  let keep = params.blur * (1.0 - state.clear) * select(1.0, 0.3, state.view == 3.0 || state.view == 7.0);
  // Skip history on a cut or with blur off, even when the frame interval is zero.
  if (keep == 0.0) {
    return vec4f(now, 1.0);
  }
  let before = textureSampleLevel(prev_trail, linear_clamp, px.uv, 0.0).rgb;
  // Per second, not per frame: the same trail at 30 fps as at 60.
  return vec4f(mix(now, before, pow(keep, rv_dt() * 60.0)), 1.0);
}
