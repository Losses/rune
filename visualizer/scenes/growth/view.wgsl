// The tower as the camera sees it, into `lit` with the distance in alpha for the depth of field:
// the plates lit by the key light with their shadows, fading into the haze; the rod of light up
// the core; dust drifting near the lens.

struct Ray {
  o: vec3f,
  d: vec3f,
}

fn camera_ray(px: Pixel) -> Ray {
  let f = normalize(state.look - state.cam_pos);
  // Across is the way round the tower, so the view never flips, even looking straight up.
  let a = state.azimuth;
  let right = vec3f(-sin(a), 0.0, cos(a));
  let up = normalize(cross(f, right));
  let p2 = rv_rotate(px.p, state.roll) * state.lens * 0.9;
  var ray: Ray;
  ray.o = state.cam_pos;
  ray.d = normalize(f + right * p2.x + up * p2.y);
  return ray;
}

// How much of the key light reaches p: the grid walked once more, towards the light.
fn sunlit(p: vec3f, n: vec3f, th: Theme) -> f32 {
  if (params.shadows <= 0.0 || dot(n, th.sun) <= 0.0) {
    return 1.0;
  }
  let h = trace(p + n * 0.02, th.sun, 40);
  return 1.0 - params.shadows * f32(h.t < 1e8);
}

fn shade_plate(p: vec3f, d: vec3f, h: Hit, th: Theme, pixel: f32) -> vec3f {
  let pl = h.plate;
  let s = surface(p, d, h.t, h, pixel);
  let n = s.n;
  var albedo = th.pale;
  if (pl.shade < -0.5) {
    albedo = th.dark;
  } else if (pl.shade > 0.75) {
    albedo = th.odd;
  } else if (pl.shade > 0.25) {
    albedo = th.mid;
  }
  // Each pour a touch lighter or darker, and the face's own clouds and streaks.
  albedo *= (0.96 + 0.08 * hash3(pl.key + 9.9)) * (0.6 + 0.8 * s.tone);
  // Deep in the tower less of the sky gets in; the key light is blocked by what really stands in
  // its way.
  let r = max(abs(p.x), abs(p.z));
  let open = smoothstep(0.0, girth_at(p.y) * 1.1, r);
  let lit = sunlit(p, h.n, th);
  let key = max(dot(n, th.sun), 0.0) * lit;
  let sky = mix(th.sky_down, th.sky_up, 0.5 + 0.5 * n.y) * mix(0.35, 0.9, open) * s.open;
  // The rod lights the faces turned to the core, the nearer the more.
  let rr = max(length(p.xz), 0.05);
  let to_rod = max(dot(n, vec3f(-p.x, 0.0, -p.z) / rr), 0.0);
  let rod = th.rod * params.rod_light * (1.0 + 3.0 * state.flash) * to_rod / (1.0 + 0.2 * rr * rr) * f32(p.y < state.top + 3.0);
  // A broad sheen: what catches the light along the rounded edges.
  let sheen = pow(max(dot(n, normalize(th.sun - d)), 0.0), 24.0) * 0.08 * lit * s.open;
  var c = albedo * (th.key * key + sky + rod) + th.key * sheen;
  // New plates flash as they land, in the white light only: under a real light a flat white plate
  // reads as unlit.
  c += mix(th.rod, vec3f(1.0), 0.5) * 2.5 * pl.fresh * params.fresh * f32(params.style == 0u);
  return c;
}

fn shade(px: Pixel) -> vec4f {
  let ray = camera_ray(px);
  let o = ray.o;
  let d = ray.d;
  let th = theme_now();
  let bg = haze(d, th);
  let h = trace(o, d, 90);
  var col = bg;
  var depth = 1000.0;
  if (h.t < 1e8) {
    let p = o + d * h.t;
    // How wide one pixel is, as an angle.
    let pixel = state.lens * 0.9 / px.size.y;
    // Concrete fades into the sky only a little, as air does.
    let fog = params.fog * select(1.0, 0.35, params.style == 1u);
    col = mix(shade_plate(p, d, h, th, pixel), bg, 1.0 - exp(-h.t * 0.007 * fog));
    depth = h.t;
  }

  // The rod of light up the core, and its glow.
  let a = dot(d.xz, d.xz);
  let b = dot(o.xz, d.xz);
  let rod = params.rod_width;
  let disc = b * b - a * (dot(o.xz, o.xz) - rod * rod);
  let bright = 1.0 + 4.0 * state.flash;
  if (disc > 0.0 && a > 1e-6) {
    let t = (-b - sqrt(disc)) / a;
    if (t > 0.0 && t < depth && o.y + d.y * t < state.top + 3.0) {
      col = th.rod * bright;
      depth = t;
    }
  }
  if (a > 1e-6) {
    let tc = max(-b / a, 0.0);
    let gap = length(o.xz + d.xz * tc);
    // In the white light the glow shows through the plates a little, as through haze; concrete
    // hides it.
    let through = select(0.35, 0.0, params.style == 1u);
    let seen = select(through, 1.0, tc < depth) * f32(o.y + d.y * tc < state.top + 3.0);
    col += th.rod * 0.8 * params.rod_glow * (0.06 + 0.5 * state.flash) * exp(-gap * 2.5) * seen;
  }

  // Dust: dark specks at a few depths in front of the lens.
  for (var k = 1.0; k <= 6.0; k += 1.0) {
    let t = k * k * 0.9 + 1.5;
    if (t > depth) {
      break;
    }
    let q = o + d * t;
    let cell = floor(q / 1.6);
    let at = (cell + vec3f(hash3(cell + 0.3), hash3(cell + 1.3), hash3(cell + 2.3))) * 1.6;
    let miss = length(cross(at - o, d));
    let size = 0.012 + 0.02 * hash3(cell + 3.3);
    if (hash3(cell + 4.3) < 0.12 * params.dust && miss < size) {
      col = mix(col, th.dust, 0.75);
      depth = t;
    }
  }
  return vec4f(col, depth);
}
