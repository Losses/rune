// This frame's layers of the tower, worked out once for all pixels (see `table_first`): each
// texel of `layers` is one part of a layer, its girth and number or one of its square plates.

fn shade(px: Pixel) -> vec4f {
  let i = u32(px.coord.y) * u32(px.size.x) + u32(px.coord.x);
  let y = table_first() + f32(i / 6u);
  let part = i % 6u;
  let r_max = layer_girth(y);
  if (part == 0u) {
    return vec4f(r_max, y, 0.0, 0.0);
  }
  return square(y, f32(part - 1u), r_max);
}
