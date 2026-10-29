// Ink lines round things, as a comic draws them: wherever the distance to what a pixel sees jumps
// from one pixel to the next (an edge in front of what is behind it: a sign before a wall, a
// building against the sky, a person against the street), a dark line `ink_width` pixels wide.
// Far off the lines thin away, or the distance would be all lines.

fn depth_at(uv: vec2f) -> f32 {
  return log(max(textureSampleLevel(street, nearest_clamp, uv, 0.0).a, 0.05));
}

fn shade(px: Pixel) -> vec4f {
  let here = textureSampleLevel(street, nearest_clamp, px.uv, 0.0);
  if (params.ink <= 0.0) {
    return vec4f(here.rgb, 1.0);
  }
  let off = params.ink_width / px.size;
  let c = log(max(here.a, 0.05));
  var jump = 0.0;
  jump = max(jump, abs(depth_at(px.uv + vec2f(off.x, 0.0)) - c));
  jump = max(jump, abs(depth_at(px.uv - vec2f(off.x, 0.0)) - c));
  jump = max(jump, abs(depth_at(px.uv + vec2f(0.0, off.y)) - c));
  jump = max(jump, abs(depth_at(px.uv - vec2f(0.0, off.y)) - c));
  // Only the nearer side of an edge is inked, so a line is one width, not two.
  let nearer = f32(depth_at(px.uv + vec2f(off.x, 0.0)) > c || depth_at(px.uv - vec2f(off.x, 0.0)) > c || depth_at(px.uv + vec2f(0.0, off.y)) > c || depth_at(px.uv - vec2f(0.0, off.y)) > c);
  let line = smoothstep(0.08, 0.25, jump) * nearer * (1.0 - smoothstep(40.0, 120.0, here.a));
  return vec4f(here.rgb * (1.0 - params.ink * line), 1.0);
}
