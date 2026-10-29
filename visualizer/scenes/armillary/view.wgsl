// The sphere in space as the camera sees it: the sky and the planet at its heart in characters,
// the rings' marks over it, and the shapes that reach this pixel laid over.

// How bright the rings, the shapes and the planet's characters are with the energy: 0.65 times in
// the quietest passage, 1.25 times in the loudest, about 0.95 at a middling one.
fn light_by_energy() -> f32 {
  return mix(0.65, 1.25, input.energy);
}

// The planet at the heart, drawn in character art: the planet's disc on the screen cut
// into character cells, rows of them anchored to it; each cell a character as dense as the planet
// is bright at the cell's middle, from the ramp . : - + = % * # @ (each drawn in a five by three
// grid of dots), lit from one side but never black on the other (a side left blank made its
// outline wander with the light), with markings of its own turning with it about a tipped axis.
// It swells with the music and on each beat; its characters stay the same size (they are cut for
// the planet at rest, so a swelling planet takes in more of them, never sliding them), and the
// grid keeps to whole pixels. Its body is dark: it hides the stars behind it, and gives no glow.
fn ascii_planet(o: vec3f, at: vec2f, px1: f32) -> vec3f {
  let c = CENTRE;
  let v = c - o;
  let zc = dot(v, state.cam_fwd);
  if (zc < 0.5) {
    return vec3f(0.0);
  }
  let s0 = round(state.focal * vec2f(dot(v, state.cam_right), dot(v, state.cam_up)) / zc / px1) * px1;
  let big = planet_radius();
  let rs = state.focal * big / zc;
  let cell = vec2f(0.6, 1.0) * 2.0 * (state.focal * params.planet / zc) / params.text_rows;
  let rel = at - s0;
  if (length(rel) > rs + cell.y * 2.0) {
    return vec3f(0.0);
  }
  let idx = floor(rel / cell);
  let hit = sphere_hit(o, view_ray(s0 + (idx + 0.5) * cell), c, big);
  if (hit.x > 1e8) {
    return vec3f(0.0);
  }
  let n = hit.yzw;
  // Lit from above to one side, a little from in front; the markings turned with the planet.
  let light = normalize(-state.cam_right * 0.55 + state.cam_up * 0.6 - state.cam_fwd * 0.6);
  let nr = turn(n, normalize(vec3f(0.35, 1.0, 0.2)), -input.planet_turn);
  let w = abs(nr) / (abs(nr.x) + abs(nr.y) + abs(nr.z));
  let marks = rv_noise(nr.yz * 2.2 + 1.3) * w.x + rv_noise(nr.zx * 2.2 + 4.1) * w.y + rv_noise(nr.xy * 2.2 + 7.7) * w.z;
  let lit = 0.24 + 0.76 * max(dot(n, light), 0.0);
  let bright = clamp(lit * (0.75 + 0.6 * marks), 0.0, 0.999);
  var ramp = array<u32, 10>(0u, 2u, 1040u, 448u, 1488u, 3640u, 21157u, 21973u, 24445u, 31719u);
  let glyph = ramp[u32(bright * 10.0)];
  // Which of its dots the pixel is in.
  let f = (rel / cell - idx - vec2f(0.12, 0.1)) / vec2f(0.76, 0.8);
  if (any(f < vec2f(0.0)) || any(f >= vec2f(1.0))) {
    return vec3f(0.0);
  }
  let gx = u32(f.x * 3.0);
  let gy = u32((1.0 - f.y) * 5.0);
  let on = (glyph >> (14u - (gy * 3u + gx))) & 1u;
  return state.palette[0].rgb * f32(on) * mix(0.55, 1.0, bright) * 0.5 * params.core * light_by_energy();
}

// The point of a circle about the core (axis ax, radius r) nearest the ray, found in a few steps:
// how far along the ray, and the point.
fn nearest_on_circle(o: vec3f, d: vec3f, ax: vec3f, r: f32) -> vec4f {
  var tc = max(dot(CENTRE - o, d), 0.0);
  var c = CENTRE;
  for (var it = 0; it < 4; it++) {
    let v = o + d * tc - CENTRE;
    let onplane = v - ax * dot(v, ax);
    c = CENTRE + onplane * (r / max(length(onplane), 1e-4));
    tc = max(dot(c - o, d), 0.0);
  }
  return vec4f(tc, c);
}

// Ring i's line (pushed out by `amp` times the spectrum) at the ray d, its neighbours dx and dy
// a pixel over and up: how much it covers the pixel (`width` pixels wide, with a little glow as
// it is `lit`), and how near its side it is (0 far, 1 near); 0 where it is not.
fn line_at(o: vec3f, d: vec3f, dx: vec3f, dy: vec3f, i: u32, r: f32, amp: f32, width: f32, lit: f32, limit: f32) -> vec2f {
  // First without the spectrum (no turning the point into an angle, no looking it up): where
  // the ray and the rays of the next pixels meet the plane, and how far from the circle.
  let ax = state.ring_z[i].xyz;
  let hc = off_circle(o, d, ax, r);
  let hr = off_circle(o, dx, ax, r);
  let ha = off_circle(o, dy, ax, r);
  if (!(hc.y > 0.5 && hc.y < limit && hr.y > 0.0 && ha.y > 0.0)) {
    return vec2f(0.0);
  }
  // The spectrum moves the line out by up to amp (see off_ring), so the ray's point is at least
  // `least` from it in the plane. (Only near the circle itself: far off, by the line where the
  // plane runs out of sight, the estimate of the distance on the screen fails.)
  let pushed = select(amp, 0.0, amp < 0.001);
  let least = max(max(hc.x - pushed * 1.0001, -hc.x), 0.0);
  if (least >= 1.0) {
    return vec2f(0.0);
  }
  // From one pixel to the next the distance to the line changes by no more than the points move
  // in the plane, and the spectrum by as fast as it can rise (a whole step of it between two of
  // its 96 points) over the angle they turn through. Where that puts the pixel further than
  // `far` pixels from the line, only the glow's tail would reach it (under a hundred-thousandth):
  // left out. At half quality and below the glow ends sooner, at half that.
  let p = o + d * hc.y - CENTRE;
  let sr = length(o + dx * hr.y - CENTRE - p);
  let sa = length(o + dy * ha.y - CENTRE - p);
  let inner = length(p) - max(sr, sa);
  if (inner > 1e-3) {
    let steep = (1.0 + pushed * 15.3 / inner) * length(vec2f(sr, sa));
    let far = select(4.0, 8.0, frame.quality > 0.6) * (2.0 + 2.0 * lit);
    if (least > far * 1.01 * max(steep, 1e-6)) {
      return vec2f(0.0);
    }
  }
  var none: RingAt;
  none.k = -1.0;
  let here = off_ring(o, d, i, r, amp, none);
  if (!(abs(here.off.x) < 1.0)) {
    return vec2f(0.0);
  }
  // (The next pixels' points mostly lie between the same two points of the spectrum.)
  let right = off_ring(o, dx, i, r, amp, here).off;
  let above = off_ring(o, dy, i, r, amp, here).off;
  let gap = abs(here.off.x) / max(length(vec2f(right.x - here.off.x, above.x - here.off.x)), 1e-6);
  let w = max(width, 1.0);
  let ink = (1.0 - smoothstep(w * 0.5, w * 0.5 + 1.0, gap)) * width / w + exp(-gap / (2.0 + 2.0 * lit)) * lit * 0.12;
  let q = o + d * here.off.y - CENTRE;
  let side = clamp(dot(q, -state.cam_fwd) / r, -1.0, 1.0);
  return vec2f(ink, side * 0.5 + 0.5);
}

// The rings drawn in marks: each a string of small round dots, or (every other ring) of
// little crosses, bigger and brighter on the near side and small and faint on the far, so the
// depth is in their size; the ecliptic's ticks short columns of dots, as many as the spectrum is loud there.
// An accent lights a ring: its dots brighten together, a little bigger and in the theme's colour,
// and dim together (`ring_on`). Each dot is found exactly: the few nearest where the ray crosses
// the ring's plane and where it passes nearest the ring, each put on the screen and measured
// there. Hidden behind the planet (further than `limit`).
fn mark(at: vec2f, b: vec3f, o: vec3f, size: f32, plus: bool, px1: f32, limit: f32) -> vec2f {
  let v = b - o;
  let zc = dot(v, state.cam_fwd);
  if (zc < 0.3 || length(v) > limit) {
    return vec2f(0.0);
  }
  let s = state.focal * vec2f(dot(v, state.cam_right), dot(v, state.cam_up)) / zc;
  let true_rad = state.focal * size / zc;
  // Never under a pixel across: smaller, it is dimmer instead. Nor more than 0.02 of the picture's
  // height in radius: a mark by the lens (the burst's cut beside a ring) would blow up into a blur;
  // marks nearer than 0.3 are left out altogether (above).
  let rad = min(max(true_rad, px1 * 0.6), 0.02);
  let dim = min(pow(true_rad / rad, 2.0), 1.0);
  let q = at - s;
  var ink: f32;
  if (plus) {
    let arm = rad * 1.7;
    let thin = max(rad * 0.45, px1 * 0.5);
    let h = (1.0 - smoothstep(thin * 0.5, thin * 0.5 + px1, abs(q.y))) * (1.0 - smoothstep(arm, arm + px1, abs(q.x)));
    let w = (1.0 - smoothstep(thin * 0.5, thin * 0.5 + px1, abs(q.x))) * (1.0 - smoothstep(arm, arm + px1, abs(q.y)));
    ink = max(h, w) * 0.8;
  } else {
    ink = 1.0 - smoothstep(rad - px1 * 0.5, rad + px1 * 0.5, length(q));
  }
  return vec2f(ink * dim, zc);
}

// How far from the ray a mark of a ring (none bigger than `size`, none further from the core than
// hi; crosses if `plus`) can be and still touch the pixel (`pixel` its width over the distance): its
// reach on the screen, as mark draws it (a dot's radius and its soft edge, a cross's arms and
// theirs, never under a pixel), taken back to the distance it is at, with a little to spare. A
// mark further from the ray draws nothing there.
fn mark_reach(o: vec3f, hi: f32, size: f32, pixel: f32, plus: bool) -> f32 {
  let foot = pixel * (length(o - CENTRE) + hi);
  let rad = max(size, 0.6 * foot);
  return select(1.02 * (rad + 0.5 * foot) + 1e-5, 2.0 * rad + 2.5 * foot, plus);
}

// Whether a dot or a tick of a ring (axis ax, its dots between radii lo and hi about the core)
// could be within `reach` of the ray o, d at all: only if the ray comes within that of the ring's
// plane while it is within that of the band between lo and hi.
fn could_mark(o: vec3f, d: vec3f, ax: vec3f, lo: f32, hi: f32, reach: f32) -> bool {
  let w = o - CENTRE;
  let den = dot(d, ax);
  let height = dot(w, ax);
  // How far from the core the ray is, |w + d t|², at its nearest and at the ends of the stretch
  // within reach of the plane.
  let wd = dot(w, d);
  let ww = dot(w, w);
  if (abs(den) < 1e-6) {
    // (Along the plane: all of the ray, or none of it.)
    return abs(height) <= reach && ww - wd * wd <= (hi + reach) * (hi + reach);
  }
  let tp = -height / den;
  let half = reach / abs(den);
  let t0 = tp - half;
  let t1 = tp + half;
  let tn = clamp(-wd, t0, t1);
  let nearest = ww + tn * (2.0 * wd + tn);
  if (nearest > (hi + reach) * (hi + reach)) {
    return false;
  }
  // (The furthest is at an end of the stretch: if both ends are inside the hole, so is all of it.)
  let low = max(lo - reach, 0.0);
  let f0 = ww + t0 * (2.0 * wd + t0);
  let f1 = ww + t1 * (2.0 * wd + t1);
  return max(f0, f1) >= low * low;
}

// What lighting adds to the rings, against the light's colour: the line of the spectrum, and the
// lit dots (`ring_on`, not dimmed by the grey look). The lens flare takes only highlights above 1
// after exposure and 1.5 times their surroundings, read at a quarter of the width, so a lit ring
// must be much brighter than it looks: measured with `probe_flare_steady` (the flare's layer, a
// threshold: none below 4 and 5, a layer of 0.14 at 5 and 6, 0.6 at 6 and 7, 1.4 at 7 and 8 on the
// strongest beats). At these values the flare comes on the bar's first beat (where the accent is
// strongest), not on the weaker beats.
const LIT_LINE: f32 = 6.0;
const LIT_DOTS: f32 = 7.0;

fn ring_marks(o: vec3f, at: vec2f, px1: f32, limit: f32) -> vec3f {
  var col = vec3f(0.0);
  let d = view_ray(at);
  let dx = view_ray(at + vec2f(px1, 0.0));
  let dy = view_ray(at + vec2f(0.0, px1));
  let c0 = CENTRE;
  for (var i = 0u; i < RINGS; i++) {
    let x = state.ring_x[i].xyz;
    let y = state.ring_y[i].xyz;
    let z = state.ring_z[i].xyz;
    let r = state.ring_x[i].w - state.ring_y[i].w * 0.5;
    let main = i == 2u;
    // The dots further apart round the outer rings (bead_gap round the ecliptic).
    let gap = params.bead_gap * pow(r / 2.2, params.gap_grow);
    let n = max(round(6.2831853 * r / gap), 12.0);
    let spec = state.ring_spec[i].x;
    let amp = spec * params.spec_amp;
    let on = state.ring_on[i];
    // (At rest the dots are half as bright as they were: they were too bright when dimmed.)
    let bright = light_by_energy();
    let rest = (select(0.075, 0.12, main) * mix(1.0, 0.3, state.layer_mix)) * params.ring_glow * bright;
    // The ring of the spectrum is a solid line in the theme's colour, the brightest thing at rest
    // and lit further where it reaches out; its dots give way to it, and lighting it lights the
    // whole line a moment.
    if (spec > 0.01) {
      let line = line_at(o, d, dx, dy, i, r, amp, 1.8, 0.4, limit);
      // (At rest 0.3 of the light's colour, dimmed by the grey look; what lighting adds is not
      // dimmed by it, and is bright enough for the lens flare to catch: see LIT_LINE.)
      col += (state.palette[1].rgb * 0.3 + state.palette[2].rgb * on.y * LIT_LINE) * line.x * spec * params.ring_glow * bright * mix(0.45, 1.0, line.y);
    }
    // (Most pixels are far from every mark of a ring: there it is not looked at. Its dots are
    // between r and r + amp from the core, the ecliptic's ticks between r - 3.6 gap and r - 0.9
    // gap; each is looked for only where it could be.)
    let lo = r + min(amp, 0.0);
    let hi = r + max(amp, 0.0);
    let biggest = params.bead_size * 1.15 * (1.0 + 0.6 * max(on.y, 0.0));
    let reach = mark_reach(o, hi, biggest, px1 / state.focal, i % 2u == 1u);
    let dots = could_mark(o, d, z, lo, hi, reach);
    var ticks = false;
    if (main) {
      ticks = could_mark(o, d, z, r - 3.6 * gap, r - 0.9 * gap, mark_reach(o, r, params.bead_size * 0.945, px1 / state.focal, false));
    }
    if (!dots && !ticks) {
      continue;
    }
    let lowest = select(lo, min(lo, r - 3.6 * gap), ticks);
    // Where to look: round the angle where the ray crosses the plane, and where it comes nearest.
    var angles = vec2f(-10.0);
    let den = dot(d, z);
    // (The second look finds only dots more than a dot and a half round from the first's; where
    // the ray crosses the plane steeply enough, any such dot is further than `reach` from it.)
    var again = true;
    var crossing = 0.0;
    if (abs(den) > 1e-4) {
      let tp = dot(c0 - o, z) / den;
      if (tp > 0.0) {
        let q = o + d * tp - c0;
        crossing = length(q);
        angles.x = atan2(dot(q, y), dot(q, x));
        again = 2.0 * sqrt(crossing * max(lowest, 0.0)) * sin(4.712389 / n) * abs(den) < reach * 1.01;
      }
    }
    if (again) {
      let near = nearest_on_circle(o, d, z, r);
      let qn = near.yzw - c0;
      angles.y = atan2(dot(qn, y), dot(qn, x));
    }
    var best = vec3f(0.0);
    var tried = vec3f(-1e9);
    for (var a = 0u; a < 2u; a++) {
      if (angles[a] < -9.0) {
        continue;
      }
      let kc = round(angles[a] / 6.2831853 * n);
      for (var dk = -1.0; dk <= 1.0; dk += 1.0) {
        let k = ((kc + dk) % n + n) % n;
        // (The second look round often finds the same dots: each is drawn once.)
        if (any(abs(tried - k) < vec3f(0.5))) {
          continue;
        }
        if (a == 0u) {
          tried[u32(dk + 1.0)] = k;
        }
        // (Only a dot with ticks, where only the ticks could reach.)
        let has_ticks = main && (u32(k) % 6u) == 0u;
        if (!dots && !(ticks && has_ticks)) {
          continue;
        }
        // (Nor one so far round from where the ray crosses the plane that, crossing it as steeply
        // as it does, the ray passes further than `reach` from it and its ticks.)
        if (angles.x > -9.0) {
          let round_off = abs(fract(k / n - angles.x / 6.2831853 + 0.5) - 0.5) * 6.2831853;
          let inmost = select(lo, lowest, has_ticks);
          if (2.0 * sqrt(crossing * max(inmost, 0.0)) * sin(round_off * 0.5) * abs(den) >= reach * 1.01) {
            continue;
          }
        }
        let u = k / n;
        let ang = u * 6.2831853;
        let out = x * cos(ang) + y * sin(ang);
        // (A mark further than `reach` from the ray draws nothing on this pixel: the dot, which the
        // spectrum moves out by up to amp, and each tick, are each looked at only nearer than that.)
        let near_dot = dots && length(cross(c0 + out * r - o, d)) - max(amp, 0.0) * 1.0001 < reach * 1.01;
        var near_ticks = false;
        if (ticks && has_ticks) {
          for (var j = 1.0; j <= 4.0; j += 1.0) {
            near_ticks = near_ticks || length(cross(c0 + out * (r - j * gap * 0.9) - o, d)) < reach * 1.01;
          }
        }
        if (!near_dot && !near_ticks) {
          continue;
        }
        // (Where the spectrum would move the dot by less than the radius can show, it is not
        // looked up.)
        var lift = 0.0;
        var loud = -1.0;
        if (amp >= r * 2.9802322e-8) {
          loud = round_spectrum(u);
          lift = amp * loud;
        }
        let b = c0 + out * (r + lift);
        // Whether this dot is in the lit part of the ring (all of it, unless `light_reach` is less).
        let from_start = min(fract(u - on.x), fract(on.x - u));
        let glow = on.y * f32(from_start <= on.w + 0.5 / n);
        let side = clamp(dot(b - c0, -state.cam_fwd) / r, -1.0, 1.0) * 0.5 + 0.5;
        let size = params.bead_size * mix(0.7, 1.15, side) * (1.0 + 0.6 * glow);
        // (Every other ring is all crosses, the rest all dots.)
        let plus = (i % 2u) == 1u;
        var m = vec2f(0.0);
        if (near_dot && length(cross(b - o, d)) < reach * 1.01) {
          m = mark(at, b, o, size, plus, px1, limit);
        }
        m.x *= 1.0 - spec;
        if (m.x > 0.0) {
          let ink = state.ring_tint[i].rgb * rest * mix(0.3, 1.0, side) + state.palette[2].rgb * glow * LIT_DOTS * bright;
          best = max(best, ink * m.x);
        }
        // The ecliptic's ticks: inward from every sixth dot, as many dots as it is loud there.
        if (near_ticks) {
          if (loud < 0.0) {
            loud = round_spectrum(u);
          }
          let count = floor(1.0 + loud * 4.0 * params.tick_spec / 0.4);
          for (var j = 1.0; j <= 4.0; j += 1.0) {
            if (j > count) {
              break;
            }
            let bt = c0 + out * (r - j * gap * 0.9);
            if (length(cross(bt - o, d)) >= reach * 1.01) {
              continue;
            }
            let mt = mark(at, bt, o, params.bead_size * 0.7 * mix(0.75, 1.35, side), false, px1, limit);
            best = max(best, state.ring_tint[i].rgb * rest * mix(0.3, 1.0, side) * (0.6 + loud) * mt.x);
          }
        }
      }
    }
    col += best;
  }
  return col;
}

// The ray through the point `at` of the screen.
fn view_ray(at: vec2f) -> vec3f {
  return normalize(state.cam_fwd * state.focal + state.cam_right * at.x + state.cam_up * at.y);
}

// The spectrum round a ring, at u (0..1 round it): mirrored, low at the start and high half way
// round, so its two ends meet.
fn round_spectrum(u: f32) -> f32 {
  return rv_spectrum(1.0 - abs(2.0 * fract(u) - 1.0), 0.0);
}

// A point met on a ring's line (see off_ring): how far off it and along the ray, and the two of
// the 96 points of the spectrum it lies between (k the first, -1 for none) and their values.
struct RingAt {
  off: vec2f,
  k: f32,
  nodes: vec2f,
}

// As off_circle, for ring i (radius r) drawn as a line of the spectrum: pushed out by `amp`
// times the spectrum at 96 points round it, straight from point to point. (`near` is a point met
// before, whose two points of the spectrum are used again if they are the same.)
fn off_ring(o: vec3f, d: vec3f, i: u32, r: f32, amp: f32, near: RingAt) -> RingAt {
  var at_ring: RingAt;
  at_ring.k = -1.0;
  let h = off_circle(o, d, state.ring_z[i].xyz, r);
  if (amp < 0.001 || h.y < 0.0) {
    at_ring.off = h;
    return at_ring;
  }
  let q = o + d * h.y - CENTRE;
  let at = fract(atan2(dot(q, state.ring_y[i].xyz), dot(q, state.ring_x[i].xyz)) / 6.2831853) * 96.0;
  let k = floor(at);
  // (Between the same two points as `near`, the spectrum is not looked up again.)
  var nodes = near.nodes;
  if (k != near.k) {
    nodes = vec2f(round_spectrum(k / 96.0), round_spectrum((k + 1.0) / 96.0));
  }
  let out = mix(nodes.x, nodes.y, at - k);
  at_ring.off = vec2f(h.x - amp * out, h.y);
  at_ring.k = k;
  at_ring.nodes = nodes;
  return at_ring;
}

// How far the ray meets the plane of the circle about the core (axis ax, radius r) from the
// circle (negative inside), and how far along the ray (-1 if it never does, or only far beyond
// the sphere, as a ray along the plane does: there the sums would overflow).
fn off_circle(o: vec3f, d: vec3f, ax: vec3f, r: f32) -> vec2f {
  let den = dot(d, ax);
  if (abs(den) < 1e-5) {
    return vec2f(0.0, -1.0);
  }
  let t = dot(CENTRE - o, ax) / den;
  if (t <= 0.0 || t > 100.0) {
    return vec2f(0.0, -1.0);
  }
  return vec2f(length(o + d * t - CENTRE) - r, t);
}

fn shade(px: Pixel) -> vec4f {
  let o = state.cam_pos;
  let d = view_ray(px.p);
  let pixel = 1.0 / (state.focal * px.size.y);

  // The planet is the only solid: dark, hiding the sky behind it (its characters are drawn over it
  // below); the rings are marks, never met as solids. The marks and the shapes are hidden behind
  // it (further than `best`).
  let best = sphere_hit(o, d, CENTRE, planet_radius()).x;
  var col = select(sky(d, pixel), state.palette[5].rgb, best < 1e8);
  col += ring_marks(o, px.p, 1.0 / px.size.y, best);
  col += ascii_planet(o, px.p, 1.0 / px.size.y);

  // The shapes: each a polygon's outline in a finer line than the rings, there only while an
  // accent has it lit, fading away; found where the ray meets its plane. Hidden behind the planet.
  let depth = best * dot(d, state.cam_fwd);
  // (Of the list of the shapes that are drawn, each passed at a glance unless this pixel is in the
  // square it reaches.)
  for (var j = 0u; j < state.hit_n; j++) {
    let h = state.hit[j];
    let apart = abs(px.p - h.xy);
    if (max(apart.x, apart.y) > h.z) {
      continue;
    }
    let i = u32(h.w);
    let sc = state.shape_scr[i];
    if (sc.w > depth) {
      continue;
    }
    let pos = state.shape_pos[i];
    let nn = state.shape_n[i];
    let den = dot(d, nn.xyz);
    if (abs(den) < 1e-3) {
      continue;
    }
    let tp = dot(pos.xyz - o, nn.xyz) / den;
    if (tp <= 0.0) {
      continue;
    }
    let q = o + d * tp - pos.xyz;
    let u = state.shape_u[i].xyz;
    let v = cross(nn.xyz, u);
    let at = vec2f(dot(q, u), dot(q, v)) / pos.w;
    // How far from the outline (corners at 1): turned into the nearest side's wedge, the
    // distance to that side; on the screen in pixels, foreshortened as the plane turns away.
    let sector = 6.2831853 / nn.w;
    let a = (fract(atan2(at.y, at.x) / sector) - 0.5) * sector;
    let w = length(at) * vec2f(cos(a), sin(a));
    let off = length(vec2f(w.x - cos(sector * 0.5), max(abs(w.y) - sin(sector * 0.5), 0.0)));
    let gap = off * pos.w / (tp * pixel) * max(abs(den), 0.3);
    let lit = state.shape_look[i].y;
    let width = 0.6 + 0.5 * lit;
    let ink = (1.0 - smoothstep(width * 0.5, width * 0.5 + 1.0, gap)) * min(width, 1.0) + exp(-gap / 2.0) * 0.06;
    // (Fading, it goes quicker once it is dim: unlit, it is not there.)
    col += state.shape_tint[i].rgb * ink * 1.6 * lit * min(lit * 2.5, 1.0) * params.shape_glow * light_by_energy();
  }
  return vec4f(col, 1.0);
}
