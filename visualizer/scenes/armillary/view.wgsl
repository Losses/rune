// The sphere in space as the camera sees it: the rings front to back, each over what is behind
// it, then the core or space behind them all; the core's glow and rays,
// the rings' haze, and the shapes that reach this pixel laid over.

struct Hit {
  t: f32,
  n: vec3f,
  // 3 ring, 5 core.
  kind: f32,
  which: u32,
}

fn consider(best: ptr<function, Hit>, h: vec4f, kind: f32, which: u32) {
  if (h.x < (*best).t) {
    (*best).t = h.x;
    (*best).n = h.yzw;
    (*best).kind = kind;
    (*best).which = which;
  }
}

// The nearest thing along the ray but the rings in `skip` (a bit for each).
fn trace(o: vec3f, d: vec3f, skip: u32) -> Hit {
  var best: Hit;
  best.t = 1e9;
  for (var i = 0u; i < RINGS; i++) {
    if ((skip & (1u << i)) == 0u) {
      consider(&best, ring_hit(i, o, d), 3.0, i);
    }
  }
  if (params.core_look == 3u) {
    consider(&best, gem_hit(o, d), 5.0, 0u);
  } else if (params.core_look == 5u) {
    consider(&best, sphere_hit(o, d, state.centre, planet_radius()), 5.0, 0u);
  } else if (params.core_look == 4u) {
    // (Drawn in lines, the gem is not met as a solid.)
  } else {
    consider(&best, sphere_hit(o, d, state.centre, core_radius()), 5.0, 0u);
  }
  return best;
}

// The core as a little star (core look 1): white-hot in the middle, darker and more orange
// towards its edge (as a star's limb is, so it reads as a ball), its surface grained with small
// cells slowly boiling.
fn star_core(n: vec3f, d: vec3f) -> vec3f {
  let mu = max(dot(n, -d), 0.0);
  let w = abs(n) / (abs(n.x) + abs(n.y) + abs(n.z));
  let f = state.core_flow;
  var grain = 0.0;
  for (var k = 0; k < 2; k++) {
    let s = 7.0 * f32(k + 1);
    grain += (rv_noise(n.yz * s + f * 0.6) * w.x + rv_noise(n.zx * s - f * 0.5) * w.y + rv_noise(n.xy * s + vec2f(f * 0.4, -f * 0.3)) * w.z) / f32(k + 1);
  }
  let limb = 1.0 - 0.7 * (1.0 - mu);
  // (Kept below the tone curve's white, which would bleach it.)
  let hot = mix(state.palette[13].rgb, core_colour(), pow(mu, 0.8));
  return hot * core_power() * 0.06 * limb * (0.92 + 0.3 * grain);
}

// The core as a lamp in a glass globe (core look 2): the globe clear, mirroring space at its
// edge, bending what is seen through it; a small bright light hanging in its middle, seen
// through the glass a little fringed with colour, each colour bent its own way (as crystal is).
fn glass_core(p: vec3f, n: vec3f, d: vec3f, pixel: f32) -> vec3f {
  let r = core_radius();
  let mu = max(dot(n, -d), 0.0);
  let fres = 0.04 + 0.96 * pow(1.0 - mu, 5.0);
  var glow = vec3f(0.0);
  var etas = array<f32, 3>(1.0 / 1.47, 1.0 / 1.5, 1.0 / 1.53);
  for (var c = 0; c < 3; c++) {
    let d1 = refract(d, n, etas[c]);
    let gap = length(p + d1 * dot(state.centre - p, d1) - state.centre) / r;
    glow[c] = exp(-gap * gap / 0.008) * 0.3 + exp(-gap / 0.25) * 0.06;
  }
  let hot = mix(core_colour(), state.palette[0].rgb, 0.6);
  // Out through the far side of the globe.
  let d1 = refract(d, n, 1.0 / 1.5);
  let p2 = p + d1 * 2.0 * dot(state.centre - p, d1);
  let n2 = normalize(p2 - state.centre);
  var d2 = refract(d1, -n2, 1.5);
  if (dot(d2, d2) < 0.5) {
    d2 = reflect(d1, -n2);
  }
  let through = sky(d2, pixel) * 0.9 + hot * glow * core_power();
  return mix(through, sky(reflect(d, n), pixel) + core_colour() * 0.03 * core_power() * pow(1.0 - mu, 2.0), fres);
}

// The core as a ball of cut crystal with the light inside it (core look 3): what its facet
// mirrors (the starry sky); a faint line along the facet's edges, as cut edges catch the light;
// and, each colour bent its own way, the light gathered on the way through: in at
// the facet, off the facets inside (wholly, past the critical angle) up to three times, gathering
// the light at the heart (small and sharp, so a facet shows it as a spark, each in its own
// place) and a little of the glow round it, and out where it can, showing the sky there. So the
// facets spark and go dark by where their paths run, and flash as the gem turns.
fn gem_core(p: vec3f, n: vec3f, d: vec3f, t: f32, pixel: f32) -> vec3f {
  let r = gem_radius();
  let f0 = 0.17;
  let fres = f0 + (1.0 - f0) * pow(1.0 - max(dot(n, -d), 0.0), 5.0);
  var col = sky(reflect(d, n), pixel) * fres;
  let hot = mix(core_colour(), state.palette[0].rgb, 0.35) * core_power() * 0.12;
  var iors = array<f32, 3>(2.36, 2.42, 2.49);
  // Quality (frame.quality, 1 is the full picture), all of Armillary's knobs: the light inside the
  // gem bounces off a facet (each one a walk over every facet, for each of three colours) three
  // times at 1 and 0.75, twice at 0.5 and 0.25, once at 0 (here); the nebula (world.wgsl) loses its
  // fine detail, half of its noises, at 0.8 and below; and the glow round the lines (below) ends
  // at half the distance at 0.6 and below. Lower levels lose the deeper reflections, the nebula's
  // fine mottling and the far tail of the glow, nothing else.
  let bounces = 1 + i32(round(2.0 * frame.quality));
  for (var c = 0; c < 3; c++) {
    let ior = iors[c];
    var dir = refract(d, n, 1.0 / ior);
    var pos = p - state.centre;
    var carry = 1.0 - fres;
    var lit = 0.0;
    var seen = 0.0;
    for (var b = 0; b < bounces; b++) {
      let ex = gem_exit(pos, dir);
      let s = clamp(dot(-pos, dir), 0.0, ex.x);
      let near = length(pos + dir * s) / r;
      lit += carry * (exp(-near * near / 0.0015) * 2.0 + ex.x / r * exp(-near / 0.2) * 0.6);
      pos += dir * ex.x;
      let out = refract(dir, -ex.yzw, ior);
      if (dot(out, out) > 0.5) {
        let fo = f0 + (1.0 - f0) * pow(1.0 - abs(dot(out, ex.yzw)), 5.0);
        seen += carry * (1.0 - fo) * sky(out, pixel)[c];
        carry *= fo;
      }
      dir = reflect(dir, ex.yzw);
    }
    col[c] += lit * hot[c] + seen;
  }
  let edge = gem_edge(p - state.centre, n);
  let foot = t * pixel;
  col += hot * 0.07 * (1.0 - smoothstep(0.4 * foot, 1.4 * foot, edge));
  return col;
}

// The core as a planet drawn in character art (core look 5): the planet's disc on the screen cut
// into character cells, rows of them anchored to it; each cell a character as dense as the planet
// is bright at the cell's middle, from the ramp . : - + = % * # @ (each drawn in a five by three
// grid of dots), lit from one side but never black on the other (a side left blank made its
// outline wander with the light), with markings of its own turning with it about a tipped axis.
// It swells with the music and on the kick; its characters stay the same size (they are cut for
// the planet at rest, so a swelling planet takes in more of them, never sliding them), and the
// grid keeps to whole pixels. Its body is dark: it hides the stars behind it, and gives no glow.
fn planet_radius() -> f32 {
  return params.planet * (0.82 + 0.35 * clamp(state.core_size - 1.0, 0.0, 0.8));
}

fn ascii_planet(o: vec3f, at: vec2f, px1: f32) -> vec3f {
  let c = state.centre;
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
  let nr = turn(n, normalize(vec3f(0.35, 1.0, 0.2)), -state.planet_turn);
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
  return state.palette[0].rgb * f32(on) * mix(0.55, 1.0, bright) * 0.5 * params.core;
}

// The colour of what the ray met (a ring met here shows only the sky through it).
fn colour_of(best: Hit, o: vec3f, d: vec3f, pixel: f32) -> vec3f {
  let p = o + d * best.t;
  let foot = best.t * pixel;
  if (best.t > 1e8) {
    return sky(d, pixel);
  }
  if (best.kind < 3.5) {
    let layer = ring_layer(best.which, p, best.n, d, foot);
    return layer.front + layer.lets * sky(d, pixel);
  }
  if (best.kind < 5.5 && params.core_look == 3u) {
    return gem_core(p, best.n, d, best.t, pixel);
  }
  if (best.kind < 5.5 && params.core_look == 5u) {
    return state.palette[5].rgb;
  }
  if (best.kind < 5.5 && params.core_look == 1u) {
    return star_core(best.n, d);
  }
  if (best.kind < 5.5 && params.core_look == 2u) {
    return glass_core(p, best.n, d, pixel);
  }
  if (best.kind < 5.5) {
    // The core: light flowing over it in slow bands, quicker with the treble, its edge brighter.
    let n = best.n;
    let flow = rv_noise(vec2f(atan2(n.z, n.x) * 1.5 + state.core_flow, n.y * 2.5 - state.core_flow * 0.6))
      + 0.5 * rv_noise(vec2f(atan2(n.z, n.x) * 3.0 - state.core_flow * 1.3, n.y * 5.0));
    let rim = pow(1.0 - max(dot(n, -d), 0.0), 2.0);
    return core_colour() * core_power() * 0.18 * (0.8 + 0.35 * flow + 0.5 * rim);
  }
  return vec3f(0.0);
}

// The point of a circle about the core (axis ax, radius r) nearest the ray, found in a few steps:
// how far along the ray, and the point.
fn nearest_on_circle(o: vec3f, d: vec3f, ax: vec3f, r: f32) -> vec4f {
  var tc = max(dot(state.centre - o, d), 0.0);
  var c = state.centre;
  for (var it = 0; it < 4; it++) {
    let v = o + d * tc - state.centre;
    let onplane = v - ax * dot(v, ax);
    c = state.centre + onplane * (r / max(length(onplane), 1e-4));
    tc = max(dot(c - o, d), 0.0);
  }
  return vec4f(tc, c);
}

// The rings drawn as lines (look 2), as a chart of the sky is: each a faint hairline along its
// middle (the ecliptic a little stronger, cut with ticks) until an accent lights it, thicker and
// bright with a little glow, fading back; fainter on the far side, so it reads which way each
// is tipped; hidden behind the core (nearer than `limit`). Each line found exactly: where the
// ray meets the ring's plane, how far that is from the ring's middle line, and (from the rays of
// the next pixels over and up) how far that is on the screen.
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
  let p = o + d * hc.y - state.centre;
  let sr = length(o + dx * hr.y - state.centre - p);
  let sa = length(o + dy * ha.y - state.centre - p);
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
  let q = o + d * here.off.y - state.centre;
  let side = clamp(dot(q, -state.cam_fwd) / r, -1.0, 1.0);
  return vec2f(ink, side * 0.5 + 0.5);
}

fn ring_lines(o: vec3f, at: vec2f, step: f32, pixel: f32, limit: f32) -> vec3f {
  var col = vec3f(0.0);
  let d = view_ray(at);
  let dx = view_ray(at + vec2f(step, 0.0));
  let dy = view_ray(at + vec2f(0.0, step));
  for (var i = 0u; i < RINGS; i++) {
    let x = state.ring_x[i];
    let y = state.ring_y[i];
    let z = state.ring_z[i].xyz;
    let r = x.w - y.w * 0.5;
    let main = i == 2u;
    let lit = state.ring_lit[i].x;
    let width = select(1.0, 1.5, main) + 1.3 * lit;
    // (While the shapes take their turn to light, the rings dim.)
    let bright = (select(0.16, 0.32, main) * mix(1.0, 0.3, state.layer_mix) + 1.6 * lit) * params.ring_glow;
    // (One ring at a time is a line of the spectrum.)
    let amp = state.ring_spec[i].x * params.spec_amp;
    let line = line_at(o, d, dx, dy, i, r, amp, width, lit, limit);
    col += state.ring_tint[i].rgb * line.x * bright * mix(0.35, 1.0, line.y);
    // The ticks, on the inside of the line, as long as the spectrum is loud round the ring (the
    // sixth ones a little longer): met where the ray crosses the ring's plane.
    let ticks = state.ring_look[i].x;
    let den = dot(d, z);
    if (main && ticks > 0.5 && abs(den) > 0.04) {
      let tp = dot(state.centre - o, z) / den;
      if (tp > 0.5 && tp < limit) {
        let q = o + d * tp - state.centre;
        let rq = length(q);
        let a = atan2(dot(q, y.xyz), dot(q, x.xyz));
        let pos = a / 6.2831853 * ticks;
        let foot = tp * pixel;
        let off = (fract(pos + 0.5) - 0.5) * 6.2831853 * rq / ticks / foot;
        let major = abs(round(pos) % 6.0) < 0.5;
        let loud = round_spectrum(round(pos) / ticks);
        let reach = 0.04 + params.tick_spec * loud + select(0.0, 0.04, major);
        let across = foot / abs(den);
        let inside = smoothstep(r - reach - across, r - reach, rq) * (1.0 - smoothstep(r, r + across, rq));
        let ink = (1.0 - smoothstep(0.4, 1.2, abs(off))) * inside * smoothstep(0.04, 0.2, abs(den));
        let side = clamp(dot(q, -state.cam_fwd) / r, -1.0, 1.0);
        col += state.ring_tint[i].rgb * ink * bright * (0.5 + loud) * mix(0.35, 1.0, side * 0.5 + 0.5);
      }
    }
  }
  return col;
}

// The rings drawn in marks (look 3): each a string of small round dots, or (every other ring) of
// little crosses, bigger and brighter on the near side and small and faint on the far, so the
// depth is in their size; the ecliptic's ticks short columns of dots, as many as the spectrum is loud there.
// An accent lights a ring: its dots switch on one after another both ways from one place, each
// at once and fully, a little bigger and in the theme's colour, and dim together. Each dot is
// found exactly: the few nearest where the ray crosses the ring's plane and where it passes
// nearest the ring, each put on the screen and measured there. Hidden behind the core (further
// than `limit`).
fn mark(at: vec2f, b: vec3f, o: vec3f, size: f32, plus: bool, px1: f32, limit: f32) -> vec2f {
  let v = b - o;
  let zc = dot(v, state.cam_fwd);
  if (zc < 0.3 || length(v) > limit) {
    return vec2f(0.0);
  }
  let s = state.focal * vec2f(dot(v, state.cam_right), dot(v, state.cam_up)) / zc;
  let true_rad = state.focal * size / zc;
  // Never under a pixel across: smaller, it is dimmer instead.
  let rad = max(true_rad, px1 * 0.6);
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
  let foot = pixel * (length(o - state.centre) + hi);
  let rad = max(size, 0.6 * foot);
  return select(1.02 * (rad + 0.5 * foot) + 1e-5, 2.0 * rad + 2.5 * foot, plus);
}

// Whether a dot or a tick of a ring (axis ax, its dots between radii lo and hi about the core)
// could be within `reach` of the ray o, d at all: only if the ray comes within that of the ring's
// plane while it is within that of the band between lo and hi.
fn could_mark(o: vec3f, d: vec3f, ax: vec3f, lo: f32, hi: f32, reach: f32) -> bool {
  let w = o - state.centre;
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

fn ring_marks(o: vec3f, at: vec2f, px1: f32, limit: f32) -> vec3f {
  var col = vec3f(0.0);
  let d = view_ray(at);
  let dx = view_ray(at + vec2f(px1, 0.0));
  let dy = view_ray(at + vec2f(0.0, px1));
  let c0 = state.centre;
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
    let rest = (select(0.15, 0.24, main) * mix(1.0, 0.3, state.layer_mix)) * params.ring_glow;
    // The ring of the spectrum is a solid line in the theme's colour, the brightest thing at rest
    // and lit further where it reaches out; its dots give way to it, and lighting it lights the
    // whole line a moment.
    if (spec > 0.01) {
      let line = line_at(o, d, dx, dy, i, r, amp, 1.8, 0.4, limit);
      col += state.palette[1].rgb * line.x * spec * (0.55 + on.z * 0.8) * params.ring_glow * mix(0.45, 1.0, line.y);
    }
    // (Most pixels are far from every mark of a ring: there it is not looked at. Its dots are
    // between r and r + amp from the core, the ecliptic's ticks between r - 3.6 gap and r - 0.9
    // gap; each is looked for only where it could be.)
    let lo = r + min(amp, 0.0);
    let hi = r + max(amp, 0.0);
    let biggest = params.bead_size * 1.15 * (1.0 + 0.6 * max(on.z, 0.0));
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
        // Whether this dot has come on: within how far the lighting has spread from where it began.
        let from_start = min(fract(u - on.x), fract(on.x - u));
        let glow = on.z * f32(from_start <= on.y + 0.5 / n);
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
          let ink = state.ring_tint[i].rgb * rest * mix(0.3, 1.0, side) + state.palette[1].rgb * glow * 2.5;
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

// The gem drawn in lines (core look 4), as the rings are: the edges of its facets, the near ones
// (where the ray goes in) bright, the far ones (where it would come out) fainter, seen through
// it; a small sharp light at its heart.
fn gem_lines(o: vec3f, d: vec3f, pixel: f32) -> vec3f {
  let h = gem_hit(o, d);
  if (h.x > 1e8) {
    return vec3f(0.0);
  }
  let p = o + d * h.x - state.centre;
  let out = gem_exit(p, d);
  let near = gem_edge(p, h.yzw) / (h.x * pixel);
  let far = gem_edge(p + d * out.x, out.yzw) / ((h.x + out.x) * pixel);
  // (A facet seen nearly edge on is faded: there the estimate of its edges on the screen fails.)
  let ink = (1.0 - smoothstep(0.5, 1.5, near)) * smoothstep(0.08, 0.3, abs(dot(h.yzw, d))) + (1.0 - smoothstep(0.5, 1.5, far)) * smoothstep(0.08, 0.3, abs(dot(out.yzw, d))) * 0.35;
  return state.palette[0].rgb * ink * 0.55 * params.ring_glow;
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
  let q = o + d * h.y - state.centre;
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
  let t = dot(state.centre - o, ax) / den;
  if (t <= 0.0 || t > 100.0) {
    return vec2f(0.0, -1.0);
  }
  return vec2f(length(o + d * t - state.centre) - r, t);
}

fn shade(px: Pixel) -> vec4f {
  let o = state.cam_pos;
  let d = view_ray(px.p);
  let pixel = 1.0 / (state.focal * px.size.y);

  // As lines or marks, the rings are not met as solids at all.
  let lines = params.look >= 2u;
  let best = trace(o, d, select(0u, (1u << RINGS) - 1u, lines));
  // Front to back through up to two rings, each laid over what is behind it, then whatever is
  // behind them.
  var col = vec3f(0.0);
  var lets = vec3f(1.0);
  var hit = best;
  var skip = 0u;
  for (var k = 0; k < 2; k++) {
    if (hit.t > 1e8 || hit.kind > 3.5) {
      break;
    }
    let layer = ring_layer(hit.which, o + d * hit.t, hit.n, d, hit.t * pixel);
    col += lets * layer.front;
    lets *= layer.lets;
    skip |= 1u << hit.which;
    hit = trace(o, d, skip);
  }
  col += lets * colour_of(hit, o, d, pixel);

  if (params.look == 2u) {
    col += ring_lines(o, px.p, 1.0 / px.size.y, pixel, best.t);
  } else if (params.look == 3u) {
    col += ring_marks(o, px.p, 1.0 / px.size.y, best.t);
  }
  if (params.core_look == 4u) {
    col += gem_lines(o, d, pixel);
  }
  if (params.core_look == 5u) {
    col += ascii_planet(o, px.p, 1.0 / px.size.y);
  }

  // A soft haze of light round each ring, in front of whatever is behind it (glass only).
  for (var i = 0u; i < RINGS * u32(params.look == 0u); i++) {
    let r = state.ring_x[i].w - state.ring_y[i].w * 0.5;
    let near = nearest_on_circle(o, d, state.ring_z[i].xyz, r);
    if (near.x > 0.5 && near.x < best.t + 0.3) {
      let gap = length(o + d * near.x - near.yzw);
      col += state.palette[2].rgb * exp(-gap / 0.22) * 0.035 * params.ring_glow;
    }
  }

  // The core's glow in the air round it, and its rays turning slowly round it (faster and
  // longer with the mids), where it is not hidden.
  let tc = max(dot(state.centre - o, d), 0.0);
  let off = o + d * tc - state.centre;
  let gap = length(off);
  // (The planet written in text has none.)
  if (tc < best.t && params.core_look != 5u) {
    let r = core_radius();
    col += core_colour() * core_power() * 0.004 / (gap * gap + 0.02) * exp(-gap * 0.6);
    // Drawn in lines, the gem's light is a small sharp point.
    if (params.core_look == 4u) {
      let small = core_radius() * 0.12;
      col += mix(core_colour(), state.palette[0].rgb, 0.6) * core_power() * 0.12 * exp(-gap * gap / (small * small));
    }
    let a = atan2(dot(off, state.cam_up), dot(off, state.cam_right));
    let rays = pow(0.5 + 0.5 * sin(a * 7.0 + state.core_rays), 12.0) + pow(0.5 + 0.5 * sin(a * 11.0 - state.core_rays * 1.6 + 1.0), 16.0);
    let reach = r * (1.0 + 1.2 * clamp(music.band_mid - 0.8, 0.0, 1.5));
    // (Only with the glass: with bronze or lines, the core is a plain light.)
    col += core_colour() * rays * smoothstep(r * 0.9, r * 1.2, gap) * exp(-(gap - r) / reach) * 0.08 * params.core * f32(params.look == 0u);
  }

  // The shapes: each a polygon's outline in a finer line than the rings, there only while an
  // accent has it lit, fading away; found where the ray meets its plane. Dimmer behind a solid
  // ring, hidden behind a solid core.
  let depth = best.t * dot(d, state.cam_fwd);
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
    var through = 1.0;
    if (sc.w > depth) {
      if (best.kind < 3.5) {
        through = 0.25;
      } else {
        continue;
      }
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
    col += state.shape_tint[i].rgb * ink * 1.6 * lit * min(lit * 2.5, 1.0) * through * params.shape_glow;
  }
  return vec4f(col, 1.0);
}
