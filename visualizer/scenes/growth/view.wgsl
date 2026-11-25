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

// The layers' table (see `table_first`): texel i of it, and where layer y is in it (-1 if it is
// not, as when the buffer is too small; then the layer is worked out here).
fn table_texel(i: u32) -> vec4f {
  let w = textureDimensions(layers).x;
  return textureLoad(layers, vec2u(i % w, i / w), 0);
}

fn table_slot(y: f32) -> i32 {
  let size = textureDimensions(layers);
  let s = y - table_first();
  if (s < 0.0 || s >= f32(size.x * size.y / 6u)) {
    return -1;
  }
  return select(-1, i32(s), table_texel(u32(s) * 6u).y == y);
}

fn table_girth(slot: i32, y: f32) -> f32 {
  if (slot < 0) {
    return layer_girth(y);
  }
  return table_texel(u32(slot) * 6u).x;
}

fn table_plates(slot: i32, y: f32, r_max: f32) -> Squares {
  if (slot < 0) {
    return layer_plates(y, r_max);
  }
  let i = u32(slot) * 6u;
  return Squares(table_texel(i + 1u), table_texel(i + 2u), table_texel(i + 3u), table_texel(i + 4u), table_texel(i + 5u), r_max);
}

// The first plate along the ray, walking the grid inside the tower's bounds, at most `steps`
// cells.
fn trace(o: vec3f, d0: vec3f, steps: i32) -> Hit {
  var hit: Hit;
  hit.t = 1e9;
  // No axis exactly zero, so every step stays finite.
  let d = select(d0, vec3f(1e-6), abs(d0) < vec3f(1e-6));
  let reach = params.girth * 1.4 * 1.6 * FILL_GIRTH.y * 1.4 + 3.0 + CELL.x;
  // Only from a little under the camera up (the camera looks up, and what is below it is let go
  // of), or the top of the tower seen from above.
  let lo = vec3f(-reach, kept_from(), -reach);
  let hi = vec3f(reach, state.top + 1.0, reach);
  let mid = 0.5 * (lo + hi);
  let size = 0.5 * (hi - lo);
  var t0 = 0.0;
  if (any(abs(o - mid) > size)) {
    let b = box_hit(o, d, mid, size);
    if (b.x > 1e8) {
      return hit;
    }
    t0 = b.x + 1e-3;
  }
  // Every plate there is now lies under the growing front: a point of one is at most half a layer
  // above its cell's middle and, with its flight, at most a cell and a half further out, so it
  // lies in the pyramid y + taper / girth * (ring - 2.5) <= top_to + 0.5 under the tip (ring the
  // distance from the axis along x or z). The walk only crosses that: the cells before it count
  // against `steps` as if walked, and it ends where the ray leaves it.
  let slope = params.taper / params.girth;
  let roof = state.top_to + 0.5;
  var t_in = t0;
  var t_out = 1e9;
  for (var side = 0; side < 5; side++) {
    // The faces: across +x, -x, +z, -z, and the roof.
    var n = vec3f(0.0, 1.0, 0.0);
    var c = roof;
    if (side < 4) {
      let s = select(-1.0, 1.0, (side & 1) == 0);
      n = select(vec3f(0.0, 1.0, s * slope), vec3f(s * slope, 1.0, 0.0), side < 2);
      c = roof + slope * 2.5;
    }
    let along = dot(n, d);
    let room = c - dot(n, o);
    if (along > 0.0) {
      t_out = min(t_out, room / along);
    } else if (along < 0.0) {
      t_in = max(t_in, room / along);
    } else if (room < 0.0) {
      return hit;
    }
  }
  if (t_in > t_out) {
    return hit;
  }
  let first = floor((o + d * t0) / CELL);
  let p = o + d * t_in;
  var cell = floor(p / CELL);
  let stp = sign(d);
  let delta = abs(CELL / d);
  var next = ((cell + max(stp, vec3f(0.0))) * CELL - o) / d;
  var layer = 1e9;
  var slot = -1;
  var r_max = 0.0;
  let passed = abs(cell - first);
  for (var k = i32(passed.x + passed.y + passed.z); k < steps; k++) {
    // A cell the tip has not grown past holds nothing yet, and no plate flies through it: most
    // of those a ray crosses are passed over at the cost of this.
    let here = (cell + 0.5) * CELL;
    if (state.top_to >= appears_at(here)) {
      if (cell.y != layer) {
        layer = cell.y;
        slot = table_slot(layer);
        r_max = table_girth(slot, layer);
      }
      // Nor does a cell beyond the layer's reach, and the one further in.
      if (max(abs(here.x), abs(here.z)) - CELL.x - 0.01 <= r_max * 1.4 + 2.0) {
        // Near the front the plate of the cell further in may still be flying out through this
        // one.
        let others = select(0.0, 1.0, arriving(cell));
        for (var j = 0.0; j <= others; j += 1.0) {
          let back = way_out(cell);
          let owner = cell - vec3f(back.x, 0.0, back.y) * j;
          if (!may_hold((owner + 0.5) * CELL, r_max)) {
            continue;
          }
          // The layer's squares are read where they are needed rather than kept, which leaves
          // more room for other pixels on the GPU.
          let pl = plate(owner, table_plates(slot, layer, r_max));
          if (pl.on > 0.5) {
            let b = box_hit(o, d, pl.centre, pl.half);
            if (b.x < hit.t) {
              hit.t = b.x;
              hit.n = b.yzw;
              hit.cell = owner;
            }
          }
        }
      }
    }
    let t_next = min(next.x, min(next.y, next.z));
    // A plate lies inside its cell, so the first one met before leaving the cell is the nearest.
    if (hit.t <= t_next) {
      break;
    }
    if (next.x < next.y && next.x < next.z) {
      cell.x += stp.x;
      next.x += delta.x;
    } else if (next.y < next.z) {
      cell.y += stp.y;
      next.y += delta.y;
    } else {
      cell.z += stp.z;
      next.z += delta.z;
    }
    let at = o + d * t_next;
    if (any(abs(at - mid) > size + CELL) || t_next > t_out) {
      break;
    }
  }
  return hit;
}

// The plate a ray met, worked out again: the walk keeps only whose it is, which keeps it light.
fn hit_plate(h: Hit) -> Plate {
  let slot = table_slot(h.cell.y);
  return plate(h.cell, table_plates(slot, h.cell.y, table_girth(slot, h.cell.y)));
}

// How much of the key light reaches p: the grid walked once more, towards the light. At a lower
// quality the walk is shorter, so only nearer plates cast shadows.
fn sunlit(p: vec3f, n: vec3f, th: Theme) -> f32 {
  if (params.shadows <= 0.0 || dot(n, th.sun) <= 0.0) {
    return 1.0;
  }
  let q = frame.quality;
  let steps = select(select(select(select(8, 12, q >= 0.25), 18, q >= 0.5), 26, q >= 0.75), 40, q >= 1.0);
  let h = trace(p + n * 0.02, th.sun, steps);
  return 1.0 - params.shadows * f32(h.t < 1e8);
}

fn shade_plate(p: vec3f, d: vec3f, h: Hit, th: Theme, pixel: f32) -> vec3f {
  // The walk towards the light first, while little else is being kept.
  let lit = sunlit(p, h.n, th);
  let pl = hit_plate(h);
  let s = surface(p, d, h.t, h, pl, pixel);
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
  let key = max(dot(n, th.sun), 0.0) * lit;
  let sky = mix(th.sky_down, th.sky_up, 0.5 + 0.5 * n.y) * mix(0.35, 0.9, open) * s.open;
  // The rod lights the faces turned to the core, the nearer the more.
  let rr = max(length(p.xz), 0.05);
  let to_rod = max(dot(n, vec3f(-p.x, 0.0, -p.z) / rr), 0.0);
  let rod = th.rod * params.rod_light * (0.4 + 1.6 * state.rod) * to_rod / (1.0 + 0.2 * rr * rr) * f32(p.y < state.top + 3.0);
  // A broad sheen: what catches the light along the rounded edges.
  let sheen = pow(max(dot(n, normalize(th.sun - d)), 0.0), 24.0) * 0.08 * lit * s.open;
  return albedo * (th.key * key + sky + rod) + th.key * sheen;
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
    // The tower fades into the sky only a little, as air does.
    col = mix(shade_plate(p, d, h, th, pixel), bg, 1.0 - exp(-h.t * 0.007 * params.fog));
    // Seen from above, the tower sinks into the haze before the depth it is drawn to, so that
    // where it stops is never seen.
    if (state.deep > 0.0) {
      col = mix(col, bg, smoothstep(0.55, 0.95, (state.top - p.y) / state.deep));
    }
    depth = h.t;
  }

  // The rod of light up the core, and its glow.
  let a = dot(d.xz, d.xz);
  let b = dot(o.xz, d.xz);
  let rod = params.rod_width;
  let disc = b * b - a * (dot(o.xz, o.xz) - rod * rod);
  let bright = 0.7 + 1.0 * state.rod;
  if (disc > 0.0 && a > 1e-6) {
    let t = (-b - sqrt(disc)) / a;
    // The rod has no end in view: it rises out of the picture, whatever the view.
    if (t > 0.0 && t < depth) {
      col = th.rod * bright;
      // The rod sinks into the haze with the tower, seen from above.
      if (state.deep > 0.0) {
        col = mix(col, bg, smoothstep(0.55, 0.95, (state.top - (o.y + d.y * t)) / state.deep));
      }
      depth = t;
    }
  }
  if (a > 1e-6) {
    let tc = max(-b / a, 0.0);
    let gap = length(o.xz + d.xz * tc);
    // The plates hide its glow.
    let seen = f32(tc < depth);
    col += th.rod * 0.8 * params.rod_glow * (0.03 + 0.15 * state.rod) * exp(-gap * 2.5) * seen;
  }

  // Dust: dark specks at a few depths in front of the lens.
  for (var k = 1.0; k <= 6.0; k += 1.0) {
    let t = k * k * 0.9 + 1.5;
    if (t > depth) {
      break;
    }
    let q = o + d * t;
    let cell = floor(q / 1.6);
    // Only a few cells hold a speck: where and how big it is is worked out only for those.
    if (hash3(cell + 4.3) >= 0.12 * params.dust) {
      continue;
    }
    let at = (cell + vec3f(hash3(cell + 0.3), hash3(cell + 1.3), hash3(cell + 2.3))) * 1.6;
    let miss = length(cross(at - o, d));
    let size = 0.012 + 0.02 * hash3(cell + 3.3);
    if (miss < size) {
      col = mix(col, th.dust, 0.75);
      depth = t;
    }
  }
  return vec4f(col, depth);
}
