// Depth of field: what is nearer or farther than the tower's core blurs, gathered from a disc of
// samples, each weighted by its own blur so sharp edges do not bleed.

fn blur_of(depth: f32) -> f32 {
  let focus = max(state.focus, 2.0);
  return clamp(params.dof * abs(depth - focus) / max(depth, 0.5), 0.0, 1.0) * 14.0;
}

fn shade(px: Pixel) -> vec4f {
  let uv = px.uv;
  let centre = textureSampleLevel(lit, linear_clamp, uv, 0.0);
  let own = blur_of(centre.a);
  if (own < 0.5 || params.dof <= 0.0) {
    return vec4f(centre.rgb, 1.0);
  }
  var sum = centre.rgb;
  var weight = 1.0;
  // Round a golden-angle spiral, each sample turned from the last; fewer of them at a lower
  // quality, for a coarser blur of the same size.
  let q = frame.quality;
  let taps = select(select(select(8.0, 12.0, q >= 0.5), 16.0, q >= 0.75), 24.0, q >= 1.0);
  let turn = vec2f(cos(2.39996), sin(2.39996));
  var dir = vec2f(1.0, 0.0);
  for (var i = 0.0; i < taps; i += 1.0) {
    let r = sqrt((i + 0.5) / taps) * own;
    let at = uv + dir * r / px.size;
    let s = textureSampleLevel(lit, linear_clamp, at, 0.0);
    let w = smoothstep(r - 1.5, r, max(blur_of(s.a), own * 0.5));
    sum += s.rgb * w;
    weight += w;
    dir = vec2f(dir.x * turn.x - dir.y * turn.y, dir.x * turn.y + dir.y * turn.x);
  }
  return vec4f(sum / weight, 1.0);
}
