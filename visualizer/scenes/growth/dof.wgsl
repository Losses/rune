// Depth of field: what is nearer or farther than the tower's core blurs, gathered from a disc of
// samples, each weighted by its own blur so sharp edges do not bleed.

fn blur_of(depth: f32) -> f32 {
  let focus = max(state.dist * 0.85, 2.0);
  return clamp(params.dof * abs(depth - focus) / max(depth, 0.5), 0.0, 1.0) * 14.0;
}

// The beat's looks, as the video has them: a warm, yellow overexposure on the kick, and now and
// then a glitch, the picture inverted to dark blue and torn into bands.
fn finish(c: vec3f, uv: vec2f) -> vec4f {
  let warm = state.kick * params.warm_kick * f32(params.style == 0u);
  var col = c * mix(vec3f(1.0), vec3f(1.12, 1.06, 0.82), warm);
  if (state.glitch > 0.01) {
    let inverted = vec3f(0.05, 0.09, 0.2) + (vec3f(1.0) - min(col, vec3f(1.0))) * vec3f(0.35, 0.48, 0.75);
    col = mix(col, inverted, state.glitch);
  }
  return vec4f(col, 1.0);
}

fn shade(px: Pixel) -> vec4f {
  var uv = px.uv;
  // A glitch tears the picture into bands sliding sideways.
  if (state.glitch > 0.01) {
    let band = floor(uv.y * 14.0);
    uv.x += (hash(vec2f(band, floor(frame.time * 30.0))) - 0.5) * 0.06 * state.glitch;
  }
  let centre = textureSampleLevel(lit, linear_clamp, uv, 0.0);
  let own = blur_of(centre.a);
  if (own < 0.5 || params.dof <= 0.0) {
    return finish(centre.rgb, uv);
  }
  var sum = centre.rgb;
  var weight = 1.0;
  for (var i = 0.0; i < 24.0; i += 1.0) {
    let r = sqrt((i + 0.5) / 24.0) * own;
    let a = i * 2.39996;
    let at = uv + vec2f(cos(a), sin(a)) * r / px.size;
    let s = textureSampleLevel(lit, linear_clamp, at, 0.0);
    let w = smoothstep(r - 1.5, r, max(blur_of(s.a), own * 0.5));
    sum += s.rgb * w;
    weight += w;
  }
  return finish(sum / weight, uv);
}
