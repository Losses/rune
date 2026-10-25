// The street with a little of the last frames left in it: motion blur that streaks the lights
// as the camera walks, none across a cut.

fn shade(px: Pixel) -> vec4f {
  let now = textureSampleLevel(inked, linear_clamp, px.uv, 0.0).rgb;
  let before = textureSampleLevel(prev_trail, linear_clamp, px.uv, 0.0).rgb;
  // High up the camera flies fast: a long trail there would show every window twice.
  let keep = params.blur * (1.0 - state.flash) * select(1.0, 0.3, state.view > 2.5);
  return vec4f(mix(now, before, keep), 1.0);
}
