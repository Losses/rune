// The camera's view, without marching anything: the sea is flat triangles, each a plane the ray
// meets in one step; the torii are boxes and cylinders the ray meets exactly. Then the light of
// the moment and the surf.

struct Ray {
  o: vec3f,
  d: vec3f,
}

fn camera_ray(px: Pixel) -> Ray {
  let f = normalize(state.cam_fwd);
  let right = normalize(cross(vec3f(0.0, 1.0, 0.0), f));
  let up = cross(f, right);
  let p2 = rv_rotate(px.p, state.roll) * state.lens * 0.8;
  var ray: Ray;
  ray.o = state.cam_pos;
  ray.d = normalize(f + right * p2.x + up * p2.y);
  return ray;
}

// How much of the sun reaches p on torii g (number i, slot as in its Hit): whether the torii
// itself is in the way.
fn sunlit(p: vec3f, n: vec3f, l: Light, strength: f32, g: Gate, i: f32, slot: f32) -> f32 {
  if (strength <= 0.0 || l.night > 0.5 || dot(n, l.sun) <= 0.0) {
    return 1.0;
  }
  // Off the surface far enough that a ray leaving it does not meet it again (no dark speckle).
  // Only the torii itself is tested: the next stands 10 m off, and casts on this one only with
  // the sun nearly down.
  let h = gate_hit(p + n * 0.08 + l.sun * 0.02, l.sun, g, i, slot);
  return 1.0 - strength * f32(h.t < 1e8);
}

// The spirits as lights: each lights what is near it, falling off with the square of the distance.
fn spirit_light(p: vec3f, n: vec3f) -> vec3f {
  var c = vec3f(0.0);
  let tint = spirit_colour();
  for (var k = 0u; k < SPIRITS; k++) {
    let s = state.spirit[k];
    if (s.w < 0.01) {
      continue;
    }
    let to = s.xyz - p;
    let d2 = dot(to, to);
    c += tint * s.w * max(dot(n, to * inverseSqrt(max(d2, 1e-4))), 0.0) / (1.0 + d2 * 0.6);
  }
  // As steady as the spirits are: the beat moves them, it does not flash their light.
  return c * params.spirit_light;
}

fn shade_torii(p: vec3f, d: vec3f, t: f32, h: Hit, l: Light, pixel: f32) -> vec3f {
  var i = h.slot;
  var g: Gate;
  if (i < 0.0) {
    i = -1.0 - i;
    g = stray(i);
    // (i is now the stray's own number, two to a slot.)
  } else {
    g = gate(i);
  }
  // Each face one flat colour, lit by how it faces the light: the facets show by the light alone.
  let n = h.n;
  let face = hash(vec2f(i * 3.7 + h.part, dot(round(n * 4.0), vec3f(1.0, 7.0, 49.0))));
  if (h.part > 3.5) {
    // A paper lantern, lit from inside.
    let glow = lantern_glow(i, l);
    // Warm paper, dim from outside so the light within shows even by day.
    let paper = vec3f(0.7, 0.5, 0.34) * (l.sun_colour * max(dot(n, l.sun), 0.0) * 0.3 + l.ambient * 0.35);
    return paper * (0.9 + 0.2 * face) + LANTERN * glow * 1.4 * (0.6 + 0.4 * max(dot(n, -d), 0.0));
  }
  let y = (p.y - g.base) / g.size;
  var albedo: vec3f;
  if (h.part > 2.5) {
    // The feet: dark stone.
    albedo = vec3f(0.11, 0.105, 0.1);
  } else if (h.part > 1.5) {
    // The kasagi's black top.
    albedo = vec3f(0.025);
  } else if (g.kind > 1.5) {
    // Old and faded: the vermilion worn to a dull rose.
    albedo = vec3f(0.36, 0.13, 0.09);
  } else if (g.kind > 0.5) {
    // Stone.
    albedo = vec3f(0.3, 0.29, 0.27);
  } else {
    albedo = vec3f(0.62, 0.07, 0.025);
    // The black foot of a vermilion pillar.
    if (h.part < 0.5 && y < 0.9) {
      albedo = vec3f(0.03);
    }
  }
  albedo *= 0.96 + 0.08 * face;
  // A ruin has weathered: its lacquer faded and darkened.
  if (g.broken > 0.5 && h.part < 2.5) {
    albedo = mix(albedo, albedo.yyy * 0.6 + vec3f(0.12, 0.08, 0.06), 0.35) * 0.85;
  }
  // A band at the water line, dark and wet.
  // A band at the water line, dark and wet, following the very triangle of water at its foot
  // (looked up only low down, where it can matter).
  if (p.y < 3.0) {
    let wf = facet_at(p.xz);
    let water = wf.centre.y - (wf.n.x * (p.x - wf.centre.x) + wf.n.z * (p.z - wf.centre.z)) / wf.n.y;
    // Higher on a ruin, which has stood in the sea longer.
    albedo *= select(1.0, 0.45, p.y - water < select(0.4, 1.1, g.broken > 0.5));
  }

  let ao = mix(0.55, 1.0, clamp(0.5 + 0.5 * n.y, 0.0, 1.0)) * mix(0.7, 1.0, smoothstep(0.0, 3.0, p.y));
  let lit = sunlit(p, n, l, params.shadows, g, i, h.slot);
  let sun = max(dot(n, l.sun), 0.0) * lit;
  // Its own lanterns light a torii of the line from its ends.
  var lamp = vec3f(0.0);
  if (h.slot >= 0.0) {
    lamp = LANTERN * lantern_glow(i, l) * 0.18 * smoothstep(1.5, 4.5, y);
  }
  let spirits_here = spirit_light(p, n);
  var c = albedo * (l.sun_colour * sun + l.ambient * (0.55 + 0.45 * n.y) * ao + lamp + spirits_here);
  // Lacquer shines a little, a whole face at a time.
  let gloss = f32(g.kind < 0.5 || g.kind > 1.5) * f32(h.part < 2.5);
  c += l.sun_colour * pow(max(dot(n, normalize(l.sun - d)), 0.0), 30.0) * 0.12 * gloss * lit;
  return c;
}

fn shade_sea(p: vec3f, d: vec3f, dist: f32, l: Light, pixel: f32, f: Facet) -> vec3f {
  // Far off, where a pixel covers much of a triangle, the triangles blend into the swell, so the
  // far sea does not flicker.
  // What one pixel covers of the water, drawn out as far as the ray grazes it (seen from the side
  // the far triangles are thin slivers, and must blend away sooner).
  let across = dist * pixel / max(-d.y, 0.01);
  let far = smoothstep(0.08, 0.35, across / f.size);
  let n = normalize(mix(f.n, swell_normal(p.xz, clamp(0.5 * across, 0.05 + dist * 0.002, 40.0)), far));
  // The torii cast no shadows on the water.
  let lit = 1.0;
  // One deep blue, the faces told apart only by the light: warm where a face turns to the sun,
  // cool where it turns away.
  let albedo = vec3f(0.02, 0.08, 0.15);
  let key = max(dot(n, l.sun), 0.0) * lit;
  var c = albedo * (l.sun_colour * key * 0.9 + l.ambient * (0.55 + 0.45 * n.y));

  // No foam: a white ring round the pillars looked pasted on. The water just round each foot is a
  // little darker, so the pillar stands in it.
  c *= mix(0.7, 1.0, smoothstep(0.0, 1.4, pillar_gap(p.xz)));
  return c;
}

// The sea in marks (sea look 0): dark water, and on it a small cross at every point of a grid
// riding the swells, lit by the light of the moment; bigger as it is nearer, never under a pixel;
// far off drawn out sideways into dashes broken like words, fewer and fewer and fading, until
// the dark water gives way to the sky's light at the horizon.
const MARK_GAP: f32 = 1.6;

fn mark_at(at: vec2f, p: vec3f, size: f32, px1: f32, key: f32) -> f32 {
  let q = project(p);
  if (q.z < 0.5) {
    return 0.0;
  }
  let focal = 1.0 / (state.lens * 0.8);
  let true_rad = focal * size / q.z;
  let rad = max(true_rad, px1 * 0.7);
  let dim = min(pow(true_rad / rad, 2.0), 1.0);
  let off = at - q.xy;
  let far = smoothstep(60.0, 160.0, q.z);
  if (far > 0.0) {
    let half = rad * mix(1.6, 5.0, far);
    if (abs(off.x) > half + px1 || abs(off.y) > rad * 0.6 + px1) {
      return 0.0;
    }
    let cell = floor((off.x + half) / (2.0 * half) * 4.0);
    let on = hash(vec2f(key, cell)) > 0.35;
    let ink = 1.0 - smoothstep(rad * 0.5, rad * 0.5 + px1, abs(off.y));
    return ink * f32(on) * mix(dim, 0.6, far);
  }
  let arm = rad * 1.6;
  let thin = max(rad * 0.4, px1 * 0.6);
  let h = (1.0 - smoothstep(thin * 0.5, thin * 0.5 + px1, abs(off.y))) * (1.0 - smoothstep(arm, arm + px1, abs(off.x)));
  let v = (1.0 - smoothstep(thin * 0.5, thin * 0.5 + px1, abs(off.x))) * (1.0 - smoothstep(arm, arm + px1, abs(off.y)));
  return max(h, v) * dim;
}

// The marks round where the ray met the swell at t (the grid's nearest nine points, each at the
// swell's height there).
fn sea_in_marks(o: vec3f, d: vec3f, t: f32, at: vec2f, px1: f32, l: Light) -> vec3f {
  let p = o + d * t;
  let c0 = round(p.xz / MARK_GAP);
  var ink = 0.0;
  for (var j = -1.0; j <= 1.0; j += 1.0) {
    for (var i = -1.0; i <= 1.0; i += 1.0) {
      let c = c0 + vec2f(i, j);
      let g = c * MARK_GAP;
      let keep = hash(c + 0.37) > smoothstep(30.0, 140.0, t) * 0.85;
      ink = max(ink, mark_at(at, vec3f(g.x, sea_h(g), g.y), params.mark_size, px1, hash(c)) * f32(keep));
    }
  }
  // The light of the moment and the dark water, both turned a little to the theme's hue.
  let light = themed(l.sun_colour * 0.6 + l.ambient * 1.6, 0.035);
  let water = themed(vec3f(0.004, 0.009, 0.016) * (0.5 + l.ambient * 2.0), 0.008);
  let c = water + light * ink * 0.55 * params.mark_glow * exp(-t / 90.0);
  // (Sampling the sky itself low down would take up the hills and the moon, in bands.)
  return mix(c, l.ambient * 0.25, smoothstep(120.0, 600.0, t));
}

fn shade(px: Pixel) -> vec4f {
  let ray = camera_ray(px);
  let o = ray.o;
  let d = ray.d;
  let l = light_now();
  // How wide one pixel is, as an angle.
  let pixel = state.lens * 0.8 / px.size.y;

  var t_sea = 1e9;
  var fac: Facet;
  let marks = params.sea_look == 0u;
  var h: Hit;
  if (marks) {
    // The torii first, as far as the ray could still be above the swell (to where it passes the
    // lowest the swell reaches): a torii met before the ray comes down to the highest the swell
    // reaches stands in front of the water wherever the swell meets the ray, which is then not
    // worked out.
    var t_top = 1e9;
    var t_low = 1e9;
    if (d.y < -1e-4) {
      // (sea_h stays within this either way.)
      let hy = abs(state.sea_amp) * SWELL_MAX * 1.3 + 0.02;
      t_top = (o.y - hy) / -d.y;
      t_low = (o.y + hy) / -d.y;
    }
    h = torii_seen(o, d, min(t_low, FAR), px.uv);
    // The sea in marks: where the ray comes down to the swell, a few steps on from the level sea.
    if (d.y < -1e-4 && !(h.t < t_top)) {
      t_sea = -o.y / d.y;
      for (var k = 0; k < 3; k++) {
        t_sea = (o.y - sea_h((o + d * t_sea).xz)) / -d.y;
      }
    }
  }
  // The low-poly sea: walking the triangles under the ray to the first it goes into.
  if (!marks && d.y < -1e-4) {
    // Beyond where a pixel covers a third of a triangle, they have blended into the swell (see
    // shade_sea).
    let sh = sea_hit(o, d, 0.35 * params.facet * max(-d.y, 0.01) / pixel);
    t_sea = sh.t;
    fac = sh.fac;
    if (t_sea > 1e8) {
      // Too far off for the walk, grazing: the plane first, then three times the plane of the
      // triangle the ray comes down on (out there the triangles blend into the swell anyway).
      t_sea = -o.y / d.y;
      for (var k = 0; k < 3; k++) {
        fac = facet_at((o + d * t_sea).xz);
        let dn = dot(d, fac.n);
        if (dn < -1e-5) {
          t_sea = dot(fac.centre - o, fac.n) / dn;
        }
      }
      // A triangle's plane runs on past the triangle: a tipped one can meet the ray nearer than
      // anything, and the sea would show through the torii in specks. No wave stands higher than
      // this, so the ray cannot meet the water before coming down to it.
      t_sea = max(t_sea, max((o.y - sea_top()) / -d.y, 0.0));
    }
  }
  if (!marks) {
    h = torii_seen(o, d, min(t_sea, FAR), px.uv);
  }

  var col: vec3f;
  var t = FAR;
  if (h.t < t_sea && h.t < FAR) {
    t = h.t;
    col = shade_torii(o + d * t, d, t, h, l, pixel);
  } else if (t_sea < 1e8) {
    t = t_sea;
    if (marks) {
      col = sea_in_marks(o, d, t, px.p, 1.0 / px.size.y, l);
    } else {
      col = shade_sea(o + d * t, d, t, l, pixel, fac);
    }
  } else {
    col = sky(d, l);
  }
  // The spirits, their ribbons and flakes, hidden behind whatever is nearer (depth along the
  // view).
  let scene_z = t * dot(d, normalize(state.cam_fwd));
  let sp = spirits(px.p, d, scene_z, 1.0 / px.size.y, l);
  col = mix(col, sp.rgb, sp.a);
  return vec4f(col, 1.0);
}

// The spirits, drawn in the world over what the camera sees, nearest first: each a ribbon of flat
// paper triangles, twisting and folding,
// lit by the sun and glowing near the head; and the flakes it sheds, tumbling down to the water
// and flashing when they turn to the sun. Returns the colour and how much it covers.

// How much of a pixel the triangle a b c covers, and its barycentric weights there; `pix` is a
// pixel's size on the screen.
fn tri_cover(p: vec2f, a: vec2f, b: vec2f, c: vec2f, pix: f32) -> vec4f {
  let area = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
  if (abs(area) < 1e-9) {
    return vec4f(0.0);
  }
  let s = sign(area);
  let w0 = ((b.x - p.x) * (c.y - p.y) - (b.y - p.y) * (c.x - p.x)) / area;
  let w1 = ((c.x - p.x) * (a.y - p.y) - (c.y - p.y) * (a.x - p.x)) / area;
  let w2 = 1.0 - w0 - w1;
  // The nearest edge, in pixels: inside positive.
  let d0 = w0 * abs(area) / max(length(c - b), 1e-6);
  let d1 = w1 * abs(area) / max(length(a - c), 1e-6);
  let d2 = w2 * abs(area) / max(length(b - a), 1e-6);
  let edge = min(d0, min(d1, d2)) / pix;
  return vec4f(clamp(edge + 0.5, 0.0, 1.0), w0, w1, w2);
}

// An equilateral triangle of radius r centred at the origin (Inigo Quilez): distance, inside
// negative.
fn sd_tri(p0: vec2f, r: f32) -> f32 {
  let k = sqrt(3.0);
  var p = vec2f(abs(p0.x) - r, p0.y + r / k);
  if (p.x + k * p.y > 0.0) {
    p = vec2f(p.x - k * p.y, -k * p.x - p.y) / 2.0;
  }
  p.x -= clamp(p.x, -2.0 * r, 0.0);
  return -length(p) * sign(p.y);
}

// Paper lit: by the sun from either side (some of it showing through), the sky, and its own
// glow.
fn paper_lit(n0: vec3f, d: vec3f, l: Light, glow: vec3f) -> vec3f {
  let n = select(n0, -n0, dot(n0, d) > 0.0);
  let paper = vec3f(0.93, 0.9, 0.84);
  let sun = max(dot(n, l.sun), 0.0) + 0.3 * max(dot(-n, l.sun), 0.0);
  var c = paper * (l.sun_colour * sun * 0.8 + l.ambient * (0.55 + 0.45 * n.y)) + glow;
  // A soft sheen where a triangle turns the sun to the camera.
  c += l.sun_colour * pow(max(dot(reflect(d, n), l.sun), 0.0), 24.0) * 0.25;
  return c;
}

fn spirits(p: vec2f, d: vec3f, scene_z: f32, pix: f32, l: Light) -> vec4f {
  let tint = spirit_colour();
  let fwd = normalize(state.cam_fwd);
  // The night lets the glow show; by day the paper's own light is little.
  let glow_k = params.trail_glow * mix(1.0, 2.2, l.night);
  var best_z = scene_z;
  var best = vec4f(0.0);
  let last = POINTS - 1u;
  for (var h = 0u; h < SPIRITS; h++) {
    let present = state.spirit[h].w;
    if (present < 0.01) {
      continue;
    }
    let b = state.bounds[h];
    if (any(p < b.xy) || any(p > b.zw)) {
      continue;
    }
    let live = mix(1.0, pow(clamp(0.2 + 0.8 * band(h % 3u), 0.2, 2.1), 1.5), params.react);
    // The ribbon, a stretch at a time.
    for (var j = 0u; j < last; j++) {
      if (j % CHUNK == 0u) {
        let cb = state.chunks[h * 8u + j / CHUNK];
        if (any(p < cb.xy) || any(p > cb.zw)) {
          j += CHUNK - 1u;
          continue;
        }
      }
      let za = state.ribbon_z[h * POINTS + j];
      let zb = state.ribbon_z[h * POINTS + j + 1u];
      if (za.z < 0.5 || zb.z < 0.5) {
        continue;
      }
      let ra = state.ribbon[h * POINTS + j];
      let rb = state.ribbon[h * POINTS + j + 1u];
      let lo = min(min(ra.xy, ra.zw), min(rb.xy, rb.zw)) - pix;
      let hi = max(max(ra.xy, ra.zw), max(rb.xy, rb.zw)) + pix;
      if (any(p < lo) || any(p > hi)) {
        continue;
      }
      for (var k = 0u; k < 2u; k++) {
        var cov: vec4f;
        var depth: f32;
        var age: f32;
        if (k == 0u) {
          cov = tri_cover(p, ra.xy, ra.zw, rb.xy, pix);
          depth = 1.0 / (cov.y / za.x + cov.z / za.y + cov.w / zb.x);
          age = (cov.y + cov.z) * za.w + cov.w * zb.w;
        } else {
          cov = tri_cover(p, ra.zw, rb.zw, rb.xy, pix);
          depth = 1.0 / (cov.y / za.y + cov.z / zb.y + cov.w / zb.x);
          age = cov.y * za.w + (cov.z + cov.w) * zb.w;
        }
        if (cov.x <= 0.0 || depth >= best_z || depth < 0.3) {
          continue;
        }
        let n = state.ribbon_n[2u * (h * POINTS + j) + k].xyz;
        // Glowing along its whole length, most near the front.
        let glow = tint * glow_k * (0.35 + 0.65 * pow(max(1.0 - age, 0.0), params.trail_fade * 3.0)) * (0.7 + 0.3 * live);
        best = vec4f(paper_lit(n, d, l, glow), cov.x * present * smoothstep(1.5, 3.0, depth));
        best_z = depth;
      }
    }
    // The flakes, a stretch at a time.
    for (var j = 2u; j < POINTS; j++) {
      if (j % CHUNK == 0u || j == 2u) {
        let cb = state.chunks[h * 8u + j / CHUNK];
        if (any(p < cb.xy) || any(p > cb.zw)) {
          j = (j / CHUNK + 1u) * CHUNK - 1u;
          continue;
        }
      }
      let fl = state.flake_n[h * POINTS + j];
      if (fl.w <= 0.0) {
        continue;
      }
      let f = state.flake[h * POINTS + j];
      let rel = p - f.xy;
      if (dot(rel, rel) > fl.w * fl.w * 1.5 || f.z >= best_z) {
        continue;
      }
      // Turned on the screen, and flattened as it turns edge-on to the camera.
      let q = rv_rotate(rel, -f.w) / vec2f(1.0, max(abs(dot(fl.xyz, d)), 0.15));
      let dist = sd_tri(q, fl.w * 0.5);
      let cov = clamp(-dist / pix + 0.5, 0.0, 1.0);
      if (cov <= 0.0) {
        continue;
      }
      let n = select(fl.xyz, -fl.xyz, dot(fl.xyz, d) > 0.0);
      var c = paper_lit(n, d, l, tint * glow_k * 0.3);
      // A flash as it turns the sun to the camera.
      c += l.sun_colour * pow(max(dot(reflect(d, n), l.sun), 0.0), 60.0) * 2.0;
      best = vec4f(c, cov * present);
      best_z = f.z;
    }
  }
  return best;
}
