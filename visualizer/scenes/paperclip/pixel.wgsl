// The picture blown up from its few pixels without smoothing, its colours cut down to a few
// levels and dithered in a 4 x 4 ordered pattern, as a picture of the nineties was.

const BAYER: array<f32, 16> = array<f32, 16>(
  0.0, 8.0, 2.0, 10.0,
  12.0, 4.0, 14.0, 6.0,
  3.0, 11.0, 1.0, 9.0,
  15.0, 7.0, 13.0, 5.0);

fn shade(px: Pixel) -> vec4f {
  let size = vec2f(textureDimensions(lo));
  let cell = floor(px.uv * size);
  let c = textureSampleLevel(lo, nearest_clamp, (cell + 0.5) / size, 0.0).rgb * exp2(params.exposure);
  let ij = vec2u(cell) % vec2u(4u);
  let threshold = (BAYER[ij.y * 4u + ij.x] + 0.5) / 16.0;
  let steps = max(params.levels - 1.0, 1.0);
  let srgb = rv_linear_to_srgb(clamp(c, vec3f(0.0), vec3f(1.0)));
  let q = floor(srgb * steps + threshold) / steps;
  return vec4f(rv_srgb_to_linear(clamp(q, vec3f(0.0), vec3f(1.0))), 1.0);
}
