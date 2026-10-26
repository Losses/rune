// What each pixel's ray meets first (find() in city.wgsl), for the street pass to draw: finding it
// and drawing it in one pass would keep too much in flight on a phone's GPU at once.

fn shade(px: Pixel) -> vec4f {
  let ray = camera_ray(px);
  let o = ray.o;
  let d = ray.d;

  // A 4×4 ordered dither, over which far small things dissolve: the Bayer matrix worked out from
  // the bits of the pixel's place (an array indexed per pixel would live in slow memory).
  let cell = vec2u(px.coord) % 4u;
  let low = cell & vec2u(1u);
  let high = cell >> vec2u(1u);
  let bayer = 8u * (low.x ^ low.y) + 4u * low.y + 2u * (high.x ^ high.y) + high.y;
  let dither = (f32(bayer) + 0.5) / 16.0;

  let tg = select(FAR, -o.y / d.y, d.y < 0.0);
  return find(o, d, ray.pix, 72.0, 28.0, min(tg, FAR), dither);
}
