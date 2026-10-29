// The street as the camera sees it. The wet ground is rough: its lights are soft highlights,
// pools drawn out towards the camera, and nothing on it mirrors the street. How far away each
// pixel is goes along in alpha, for the ink lines drawn round things (ink.wgsl).

fn shade(px: Pixel) -> vec4f {
  let ray = camera_ray(px);
  let o = ray.o;
  let d = ray.d;
  let pix = ray.pix;
  let len = length(d);

  // Quality (frame.quality, 1 to 0; the picture keeps its composition at every level). What each
  // level drops: small things and people dissolve nearer (hit_street: params.detail down to 0.6
  // of it at 0, in hit.wgsl's pass), the lit street looks at one row of shops and signs instead of
  // two below 0.5 (street_light), and the clouds lose octaves: 4 at 0.75 and above, 3 below, 2
  // below 0.25 (sky). The grid walk and the rows over the street always take their full 72 and 28.
  let tg = select(FAR, -o.y / d.y, d.y < 0.0);
  let hit = scene(o, d, pix, min(tg, FAR), textureLoad(seen, vec2i(px.coord), 0));
  var col = hit.rgb;
  var tmax = hit.w;
  if (d.y < 0.0 && tg <= hit.w) {
    tmax = tg;
    let p = o + d * tg;
    let dist = tg * len;
    let down = -d.y / len;
    let g = ground(p, dist * pix / max(down, 0.04));
    // Puddles are smoother than the bricks around them, their highlights tighter.
    let puddle = smoothstep(0.4, 0.8, g[1].z);
    // Only the puddles mirror: the dry street is matte, and its sheen is not worked out at all.
    var gloss = vec3f(0.0);
    if (puddle > 0.0) {
      let rough = params.roughness * (1.0 - 0.6 * puddle);
      let normal = normalize(vec3f(g[1].x, 1.0, g[1].y));
      gloss = street_sheen(p, -d / len, normal, rough) * puddle * g[1].z * params.sheen;
    }
    // Wet ground is darker: what it mirrors shows against it.
    col = g[0] * ground_light(p) * (1.0 - params.wet_dark * g[1].z) + gloss;
    col = fogged(col, dist, d / len);
  }
  let lanterns = strings(o, d, tmax, pix);
  col = mix(col, lanterns.rgb + vec3f(0.01, 0.008, 0.012), lanterns.a);
  col *= 1.0 - 0.9 * wires(o, d, tmax, pix);
  return vec4f(col, tmax * len);
}
