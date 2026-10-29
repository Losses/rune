// The sea of torii, shared by the passes: a reef winding out to the horizon with a torii every
// few metres along it, the open sea with its rocks around, and the sky with its light. All in
// metres and linear HDR: x across, y up, z along the reef.

const FAR: f32 = 1500.0;
const PI: f32 = 3.14159265;

// The reef's line (path_x) and hash are in state.wgsl, which the update shares.

// ---- The torii ----

// Where each torii stands (Gate, gate, stray) is in state.wgsl, which the camera shares.
// Every shape here is met by its ray in one go (boxes and cylinders, solved exactly): nothing is
// marched, so the cost of a pixel is a few dozen small sums.

// What a ray met: how far, its normal, and what it is (part: 0 pillar, 1 beam, 2 the black cap,
// 3 the feet; the slot, as -1 - slot for a stray).
struct Hit {
  t: f32,
  n: vec3f,
  part: f32,
  slot: f32,
}

fn no_hit() -> Hit {
  var h: Hit;
  h.t = 1e9;
  h.n = vec3f(0.0, 1.0, 0.0);
  h.part = -1.0;
  h.slot = 0.0;
  return h;
}

// A box centred at c with half sizes s (Inigo Quilez's slab method): distance and normal.
fn box_hit(ro: vec3f, rd: vec3f, c: vec3f, s: vec3f) -> vec4f {
  let m = 1.0 / rd;
  let n = m * (ro - c);
  let k = abs(m) * s;
  let t1 = -n - k;
  let t2 = -n + k;
  let near = max(max(t1.x, t1.y), t1.z);
  let far = min(min(t2.x, t2.y), t2.z);
  if (near > far || far < 0.0 || near < 0.0) {
    return vec4f(1e9, 0.0, 1.0, 0.0);
  }
  let nrm = -sign(rd) * step(t1.yzx, t1.xyz) * step(t1.zxy, t1.xyz);
  return vec4f(near, nrm);
}

// Whether a ray meets the box centred at c with half sizes s anywhere ahead (from inside it too):
// what a part must pass before it is worked out exactly.
fn meets_box(ro: vec3f, rd: vec3f, c: vec3f, s: vec3f) -> bool {
  let m = 1.0 / select(rd, vec3f(1e-9), abs(rd) < vec3f(1e-9));
  let n = m * (ro - c);
  let k = abs(m) * s;
  let far = min(min(-n.x + k.x, -n.y + k.y), -n.z + k.z);
  return max(max(-n.x - k.x, -n.y - k.y), -n.z - k.z) <= far && far >= 0.0;
}

// Whether a ray meets the ball round c of radius r anywhere ahead.
fn meets_ball(ro: vec3f, rd: vec3f, c: vec3f, r: f32) -> bool {
  let oc = ro - c;
  let b = dot(oc, rd);
  let cc = dot(oc, oc) - r * r;
  return cc <= 0.0 || (b <= 0.0 && b * b - cc * dot(rd, rd) >= 0.0);
}

// A pillar: an upright hexagonal prism at x = cx (z = 0), its faces r from the axis, from y0 to
// y1: three pairs of sides and its top and foot, the faces of a convex shape met in one go.
fn pillar_hit(ro: vec3f, rd: vec3f, cx: f32, r: f32, y0: f32, y1: f32) -> vec4f {
  let o = ro - vec3f(cx, 0.0, 0.0);
  var near = -1e9;
  var far = 1e9;
  var nn = vec3f(0.0, 1.0, 0.0);
  for (var k = 0u; k < 4u; k++) {
    var n = vec3f(0.0, 1.0, 0.0);
    var lo = y0;
    var hi = y1;
    if (k < 3u) {
      let a = f32(k) * 1.0471976 + 0.5235988;
      n = vec3f(cos(a), 0.0, sin(a));
      lo = -r;
      hi = r;
    }
    let den = dot(n, rd);
    let at = dot(n, o);
    if (abs(den) < 1e-7) {
      if (at < lo || at > hi) {
        return vec4f(1e9, 0.0, 1.0, 0.0);
      }
      continue;
    }
    var t0 = (lo - at) / den;
    var t1 = (hi - at) / den;
    var n0 = -n;
    if (t0 > t1) {
      let tt = t0;
      t0 = t1;
      t1 = tt;
      n0 = n;
    }
    if (t0 > near) {
      near = t0;
      nn = n0;
    }
    far = min(far, t1);
  }
  if (near > far || far < 0.0 || near < 0.0) {
    return vec4f(1e9, 0.0, 1.0, 0.0);
  }
  return vec4f(near, nn);
}

// Clip a ray against one plane (inside where dot(n, x) <= d), narrowing where it is inside the
// shape so far; the normal of the face it enters by is kept. False if it misses altogether.
fn clip(near: ptr<function, f32>, far: ptr<function, f32>, nn: ptr<function, vec3f>, o: vec3f, dir: vec3f, n: vec3f, d: f32) -> bool {
  let den = dot(n, dir);
  let room = d - dot(n, o);
  if (abs(den) < 1e-7) {
    return room >= 0.0;
  }
  let t = room / den;
  if (den < 0.0) {
    if (t > *near) {
      *near = t;
      *nn = n;
    }
  } else {
    *far = min(*far, t);
  }
  return true;
}

// A tapered pillar: an upright hexagonal prism round x = cx, its faces rb from the axis at y0 and
// rt at y1 (narrowing as it rises), with its top (level, or tipped by `slant` where it has broken
// off) and foot; the faces of a convex shape met in one go.
fn tapered_pillar(ro: vec3f, rd: vec3f, cx: f32, rb: f32, rt: f32, y0: f32, y1: f32, slant: vec2f) -> vec4f {
  let o = ro - vec3f(cx, 0.0, 0.0);
  var near = -1e9;
  var far = 1e9;
  var nn = vec3f(0.0, 1.0, 0.0);
  // The top: level, or (broken off) a slanting face through its middle.
  let top = normalize(vec3f(slant.x, 1.0, slant.y));
  var ok = clip(&near, &far, &nn, o, rd, top, top.y * y1) && clip(&near, &far, &nn, o, rd, vec3f(0.0, -1.0, 0.0), -y0);
  let lean = (rb - rt) / max(y1 - y0, 1e-3);
  for (var k = 0u; k < 6u; k++) {
    let a = f32(k) * 1.0471976 + 0.5235988;
    let side = vec3f(cos(a), 0.0, sin(a));
    let n = normalize(side + vec3f(0.0, lean, 0.0));
    ok = ok && clip(&near, &far, &nn, o, rd, n, dot(n, side * rb + vec3f(0.0, y0, 0.0)));
  }
  if (!ok || near > far || far < 0.0 || near < 0.0) {
    return vec4f(1e9, 0.0, 1.0, 0.0);
  }
  return vec4f(near, nn);
}

// A box tipped up by angle a (about z) round c, with half sizes hs: distance and normal.
fn tipped_box(ro: vec3f, rd: vec3f, c: vec3f, hs: vec3f, a: f32) -> vec4f {
  let o2 = ro - c;
  let ro2 = vec3f(o2.x * cos(a) + o2.y * sin(a), -o2.x * sin(a) + o2.y * cos(a), o2.z);
  let rd2 = vec3f(rd.x * cos(a) + rd.y * sin(a), -rd.x * sin(a) + rd.y * cos(a), rd.z);
  let h = box_hit(ro2, rd2, vec3f(0.0), hs);
  return vec4f(h.x, h.y * cos(a) - h.z * sin(a), h.y * sin(a) + h.z * cos(a), h.w);
}

// A box turned about the upright by yaw, then tipped about its length by tilt, round c, with half
// sizes hs: distance and normal. For the pieces of a ruin lying in the water.
fn lying_box(ro: vec3f, rd: vec3f, c: vec3f, hs: vec3f, yaw: f32, tilt: f32) -> vec4f {
  let o = ro - c;
  let cy = cos(yaw);
  let sy = sin(yaw);
  let o1 = vec3f(o.x * cy - o.z * sy, o.y, o.x * sy + o.z * cy);
  let d1 = vec3f(rd.x * cy - rd.z * sy, rd.y, rd.x * sy + rd.z * cy);
  let h = tipped_box(o1, d1, vec3f(0.0), hs, tilt);
  let n1 = h.yzw;
  return vec4f(h.x, n1.x * cy + n1.z * sy, n1.y, -n1.x * sy + n1.z * cy);
}

fn keep(best: ptr<function, vec4f>, part: ptr<function, f32>, h: vec4f, which: f32) {
  if (h.x < (*best).x) {
    *best = h;
    *part = which;
  }
}

// A torii met by the ray: into its frame (facing along the line, leaning, its size), the parts
// there, and the normal back.
fn gate_hit(o: vec3f, d: vec3f, g: Gate, i: f32, slot: f32) -> Hit {
  var h = no_hit();
  // A sphere round the whole torii first, in the world, before turning into its frame: most rays
  // pass it by.
  let oc = o - vec3f(g.x, g.base + 3.0 * g.size, g.z);
  let rr = 5.0 * g.size;
  let bb = dot(oc, d);
  let cc = dot(oc, oc) - rr * rr;
  if (cc > 0.0 && (bb > 0.0 || bb * bb - cc * dot(d, d) < 0.0)) {
    return h;
  }
  let ya = atan(path_dx(g.z)) + g.turn;
  let rel = (o - vec3f(g.x, g.base, g.z)) / g.size;
  var ro = vec3f(rel.x * cos(ya) - rel.z * sin(ya), rel.y, rel.x * sin(ya) + rel.z * cos(ya));
  var rd = vec3f(d.x * cos(ya) - d.z * sin(ya), d.y, d.x * sin(ya) + d.z * cos(ya));
  let la = g.lean;
  ro = vec3f(ro.x * cos(la) - ro.y * sin(la), ro.x * sin(la) + ro.y * cos(la), ro.z);
  rd = vec3f(rd.x * cos(la) - rd.y * sin(la), rd.x * sin(la) + rd.y * cos(la), rd.z);
  var best = vec4f(1e9, 0.0, 1.0, 0.0);
  var part = -1.0;
  part_hits(ro, rd, g, i, &best, &part);
  // The line's torii that still have their kasagi carry a lantern under each end of it, clear
  // of the camera's way, tipped with it if it has slipped. They sway slowly on their cords along
  // the line, each its own way and pace.
  if (slot >= 0.0 && params.lanterns > 0.0 && g.broken < 0.5 && g.lod < 0.9) {
    let sk = g.askew;
    let rk = vec3f(ro.x * cos(sk) + (ro.y - 5.86) * sin(sk), -ro.x * sin(sk) + (ro.y - 5.86) * cos(sk) + 5.86, ro.z);
    let dk = vec3f(rd.x * cos(sk) + rd.y * sin(sk), -rd.x * sin(sk) + rd.y * cos(sk), rd.z);
    for (var k = -1.0; k <= 1.0; k += 2.0) {
      // However it swings, a lantern stays within a ball round where its cord hangs from.
      if (!meets_ball(rk, dk, vec3f(3.1 * k, 5.66, 0.0), 1.15)) {
        continue;
      }
      let sw = params.lantern_swing * sin(frame.time * (0.5 + 0.25 * hash(vec2f(i, k + 3.0))) + i * 1.3 + k * 0.9);
      let cs = cos(sw);
      let sn = sin(sw);
      // Into the frame of the lantern swinging from its cord under the kasagi.
      let q = rk - vec3f(3.1 * k, 5.66, 0.0);
      let qo = vec3f(q.x, q.y * cs + q.z * sn, -q.y * sn + q.z * cs);
      let qd = vec3f(dk.x, dk.y * cs + dk.z * sn, -dk.y * sn + dk.z * cs);
      let lh = pillar_hit(qo - vec3f(0.0, -0.71, 0.0), qd, 0.0, 0.22, -0.32, 0.32);
      // The normal back: unswing, then untip.
      let sn1 = vec3f(lh.y, lh.z * cs - lh.w * sn, lh.z * sn + lh.w * cs);
      keep(&best, &part, vec4f(lh.x, sn1.x * cos(sk) - sn1.y * sin(sk), sn1.x * sin(sk) + sn1.y * cos(sk), sn1.z), 4.0);
    }
  }
  if (best.x > 1e8) {
    return h;
  }
  // The normal back to the world: undo the lean, then the facing.
  var n = best.yzw;
  n = vec3f(n.x * cos(la) + n.y * sin(la), -n.x * sin(la) + n.y * cos(la), n.z);
  n = vec3f(n.x * cos(ya) + n.z * sin(ya), n.y, -n.x * sin(ya) + n.z * cos(ya));
  h.t = best.x * g.size;
  h.n = normalize(n);
  h.part = part;
  h.slot = slot;
  return h;
}

fn part_hits(ro: vec3f, rd: vec3f, g: Gate, i: f32, best: ptr<function, vec4f>, part: ptr<function, f32>) {
  let b = g.broken;
  // A box round the whole torii first: most rays miss it (unless the camera is inside). A ruin's
  // box reaches out to the pieces lying in the water beside it.
  let span = select(vec3f(3.6, 3.4, 0.5), vec3f(4.4, 3.4, 3.4), b > 1.5);
  let inside = all(abs(ro - vec3f(0.0, 3.0, 0.0)) < span);
  if (!inside && box_hit(ro, rd, vec3f(0.0, 3.0, 0.0), span).x > 1e8) {
    return;
  }
  // Nor does a ray that goes through the opening meet anything of a torii not fallen (looking
  // down the line, most rays pass a long way through gate after gate): where it goes into the
  // box and where it leaves it, it is between the pillars and below the nuki, and it neither
  // comes up into the box from below nor leaves it downwards (the pillars and feet reach on below
  // the box).
  if (b < 1.5) {
    let m = 1.0 / select(rd, vec3f(1e-9), abs(rd) < vec3f(1e-9));
    let n = m * (ro - vec3f(0.0, 3.0, 0.0));
    let k = abs(m) * span;
    let into = ro + rd * max(max(max(-n.x - k.x, -n.y - k.y), -n.z - k.z), 0.0);
    let out = ro + rd * min(min(-n.x + k.x, -n.y + k.y), -n.z + k.z);
    if (max(abs(into.x), abs(out.x)) < 1.38 && max(into.y, out.y) < 4.26 && min(into.y, out.y) > -0.39) {
      return;
    }
  }
  // A clean low-poly torii: tapered hexagonal pillars on wider feet, straight beams, and a kasagi
  // in five pieces rising to its ends. Ruins come in three kinds: 1 the kasagi snapped, half of
  // it gone and the broken piece hanging; 2 the top fallen, one pillar broken lower down, the nuki
  // hanging from the other and a beam lying in the water; 3 only stumps, broken off at slants,
  // the beams lying in the water.
  let side = select(-1.0, 1.0, hash(vec2f(i, 31.7)) < 0.5);
  var top_l = 5.6;
  var top_r = 5.6;
  var cut_l = vec2f(0.0);
  var cut_r = vec2f(0.0);
  if (b > 2.5) {
    top_l = 1.0 + 3.0 * hash(vec2f(i, 1.1));
    top_r = 1.0 + 3.0 * hash(vec2f(i, 2.2));
    cut_l = (vec2f(hash(vec2f(i, 3.3)), hash(vec2f(i, 4.4))) - 0.5) * 1.6;
    cut_r = (vec2f(hash(vec2f(i, 5.5)), hash(vec2f(i, 6.6))) - 0.5) * 1.6;
  } else if (b > 1.5) {
    let low = 2.8 + 1.6 * hash(vec2f(i, 7.7));
    let cut = (vec2f(hash(vec2f(i, 8.8)), hash(vec2f(i, 9.9))) - 0.5) * 1.4;
    if (side > 0.0) {
      top_r = low;
      cut_r = cut;
    } else {
      top_l = low;
      cut_l = cut;
    }
  }
  // Seen small, plainly: square pillars, the nuki and the kasagi each one straight box.
  if (g.lod > 0.5) {
    keep(best, part, box_hit(ro, rd, vec3f(-2.0, 0.5 * (top_l - 3.0), 0.0), vec3f(0.27, 0.5 * (top_l + 3.0), 0.27)), 0.0);
    keep(best, part, box_hit(ro, rd, vec3f(2.0, 0.5 * (top_r - 3.0), 0.0), vec3f(0.27, 0.5 * (top_r + 3.0), 0.27)), 0.0);
    if (b < 1.5) {
      keep(best, part, box_hit(ro, rd, vec3f(0.0, 4.45, 0.0), vec3f(2.75, 0.17, 0.11)), 1.0);
      keep(best, part, box_hit(ro, rd, vec3f(0.0, 5.75, 0.0), vec3f(3.3, 0.3, 0.3)), 1.0);
      // (Drawn plainly only for the quality, its kasagi's top still black.)
      if (g.lod < 0.9 && *part == 1.0 && (*best).z > 0.6 && ro.y + rd.y * (*best).x > 5.9) {
        *part = 2.0;
      }
    }
    return;
  }
  // Each pillar stands with its foot in an upright box (reaching up past its top as far as a
  // slanting break can): a ray that misses the box misses both.
  let up_l = top_l + 0.35 * (abs(cut_l.x) + abs(cut_l.y)) + 0.05;
  let up_r = top_r + 0.35 * (abs(cut_r.x) + abs(cut_r.y)) + 0.05;
  let near_l = meets_box(ro, rd, vec3f(-2.0, 0.5 * (up_l - 3.05), 0.0), vec3f(0.62, 0.5 * (up_l + 3.05), 0.62));
  let near_r = meets_box(ro, rd, vec3f(2.0, 0.5 * (up_r - 3.05), 0.0), vec3f(0.62, 0.5 * (up_r + 3.05), 0.62));
  if (near_l) {
    keep(best, part, tapered_pillar(ro, rd, -2.0, 0.29, 0.23, -3.0, top_l, cut_l), 0.0);
  }
  if (near_r) {
    keep(best, part, tapered_pillar(ro, rd, 2.0, 0.29, 0.23, -3.0, top_r, cut_r), 0.0);
  }
  if (near_l) {
    keep(best, part, tapered_pillar(ro, rd, -2.0, 0.5, 0.36, -3.0, 0.45, vec2f(0.0)), 3.0);
  }
  if (near_r) {
    keep(best, part, tapered_pillar(ro, rd, 2.0, 0.5, 0.36, -3.0, 0.45, vec2f(0.0)), 3.0);
  }
  // The nuki: across; or, the top fallen, hanging down from the pillar still standing.
  if (b < 1.5) {
    keep(best, part, box_hit(ro, rd, vec3f(0.0, 4.45, 0.0), vec3f(2.75, 0.17, 0.11)), 1.0);
  } else if (b < 2.5) {
    let a = 0.5;
    keep(best, part, tipped_box(ro, rd, vec3f(-side * 2.0 + side * 2.3 * cos(a), 4.45 - 2.3 * sin(a), 0.0), vec3f(2.3, 0.17, 0.11), -side * a), 1.0);
  }
  if (b < 1.5) {
    let half = b > 0.5;
    if (half) {
      // Snapped: the shimaki and kasagi on one side only, the broken piece hanging at the break.
      keep(best, part, box_hit(ro, rd, vec3f(side * 1.45, 5.5, 0.0), vec3f(1.5, 0.13, 0.17)), 1.0);
      keep(best, part, box_hit(ro, rd, vec3f(side * 0.55, 5.86, 0.0), vec3f(0.55, 0.2, 0.3)), 1.0);
      keep(best, part, tipped_box(ro, rd, vec3f(1.83 * side, 5.9, 0.0), vec3f(0.76, 0.2, 0.3), 0.06 * side), 1.0);
      keep(best, part, tipped_box(ro, rd, vec3f(2.95 * side, 6.04, 0.0), vec3f(0.42, 0.21, 0.31), 0.2 * side), 1.0);
      keep(best, part, tipped_box(ro, rd, vec3f(-side * 0.3, 5.55, 0.02), vec3f(0.34, 0.18, 0.27), side * 0.95), 1.0);
    } else {
      keep(best, part, box_hit(ro, rd, vec3f(0.0, 5.5, 0.0), vec3f(2.95, 0.13, 0.17)), 1.0);
      keep(best, part, box_hit(ro, rd, vec3f(0.0, 4.98, 0.0), vec3f(0.16, 0.38, 0.08)), 1.0);
      // A slipped kasagi is tipped as a whole about its middle, one end down.
      let sk = g.askew;
      let rk = vec3f(ro.x * cos(sk) + (ro.y - 5.86) * sin(sk), -ro.x * sin(sk) + (ro.y - 5.86) * cos(sk) + 5.86, ro.z);
      let dk = vec3f(rd.x * cos(sk) + rd.y * sin(sk), -rd.x * sin(sk) + rd.y * cos(sk), rd.z);
      var kb = vec4f(1e9, 0.0, 1.0, 0.0);
      var kp = -1.0;
      // Its five pieces, all within one box.
      if (meets_box(rk, dk, vec3f(0.0, 5.95, 0.0), vec3f(3.5, 0.5, 0.36))) {
        keep(&kb, &kp, box_hit(rk, dk, vec3f(0.0, 5.86, 0.0), vec3f(1.1, 0.2, 0.3)), 1.0);
        for (var k = -1.0; k <= 1.0; k += 2.0) {
          keep(&kb, &kp, tipped_box(rk, dk, vec3f(1.83 * k, 5.9, 0.0), vec3f(0.76, 0.2, 0.3), 0.06 * k), 1.0);
          keep(&kb, &kp, tipped_box(rk, dk, vec3f(2.95 * k, 6.04, 0.0), vec3f(0.42, 0.21, 0.31), 0.2 * k), 1.0);
        }
      }
      if (kb.x < 1e8) {
        let kn = vec3f(kb.y * cos(sk) - kb.z * sin(sk), kb.y * sin(sk) + kb.z * cos(sk), kb.w);
        keep(best, part, vec4f(kb.x, kn), 1.0);
      }
    }
    // The kasagi's top is black.
    if (*part == 1.0 && (*best).z > 0.6 && ro.y + rd.y * (*best).x > 5.9) {
      *part = 2.0;
    }
  }
  // What fell, lying in the water beside the ruin: a length of kasagi, and for stumps the nuki too.
  if (b > 1.5) {
    let water = -g.base / g.size;
    let h1 = hash(vec2f(i, 41.3));
    let h2 = hash(vec2f(i, 42.9));
    let zs = select(-1.0, 1.0, h2 < 0.5);
    keep(best, part, lying_box(ro, rd, vec3f(side * (0.8 + 0.8 * h1), water + 0.04, zs * (1.6 + 0.8 * h2)), vec3f(2.2, 0.2, 0.3), (h1 - 0.5) * 1.4, (h2 - 0.5) * 0.35), 1.0);
    if (b > 2.5) {
      keep(best, part, lying_box(ro, rd, vec3f(-side * (0.6 + h2), water + 0.02, -zs * (1.4 + 0.9 * h1)), vec3f(2.0, 0.17, 0.11), (h2 - 0.5) * 2.0, (h1 - 0.5) * 0.3), 1.0);
    }
  }
}

// The nearest torii a camera ray meets before tmax: only those listed for its tile of the screen
// (at uv), whose bounding spheres cover it.
fn torii_seen(o: vec3f, d: vec3f, tmax: f32, uv: vec2f) -> Hit {
  var best = no_hit();
  let tx = min(u32(uv.x * f32(TX)), TX - 1u);
  let ty = min(u32(uv.y * f32(TY)), TY - 1u);
  let k = ty * TX + tx;
  let count = state.tile_n[k];
  // Below full quality, torii a little bigger on the screen than the update draws plainly are
  // drawn plainly too (as a few boxes, keeping their lanterns: lod 0.75): from under about a
  // tenth of the screen's height across at half quality to under a sixth at the least.
  let plain = select(0.0, mix(0.17, 0.11, clamp(frame.quality * 2.0, 0.0, 1.0)), frame.quality < 0.75);
  let focal = 1.0 / (state.lens * 0.8);
  for (var j = 0u; j < count; j++) {
    let id = state.tile[k * LIST + j];
    let b = state.torii_b[id];
    // (Nothing of it is nearer than this: met something nearer already, or the water, pass it
    // by.)
    if (b.w > min(best.t, tmax)) {
      continue;
    }
    // Its bounding ball (as gate_hit tests it first) before the rest of it is read: most rays
    // pass it by.
    let a = state.torii_a[id];
    let oc = o - vec3f(a.w, b.y + 3.0 * a.z, b.x);
    let rr = 5.0 * a.z;
    let bb = dot(oc, d);
    let cc = dot(oc, oc) - rr * rr;
    if (cc > 0.0 && (bb > 0.0 || bb * bb - cc * dot(d, d) < 0.0)) {
      continue;
    }
    let c = state.torii_c[id];
    var g = unpack_gate(a, b, c);
    if (g.lod < 0.5 && focal * 7.2 * g.size < plain * (b.w + 5.0 * g.size)) {
      g.lod = 0.75;
    }
    let h = gate_hit(o, d, g, select(-1.0 - c.w, c.w, c.w >= 0.0), c.w);
    if (h.t < best.t) {
      best = h;
    }
  }
  if (best.t > tmax) {
    return no_hit();
  }
  return best;
}

// The nearest pillar standing in the water to xz: how far from its foot's edge, the bearing
// round it (radians), and a number of its own; the foot is about 0.38 of a torii's size across
// at the water line.
fn pillar_near(xz: vec2f) -> vec3f {
  let i = round(xz.y / params.spacing);
  var best = vec3f(1e9, 0.0, 0.0);
  for (var w = 0.0; w < 3.0; w += 1.0) {
    var g = gate(i);
    if (w > 0.5) {
      g = stray(2.0 * i + w - 1.0);
    }
    if (g.here < 0.5) {
      continue;
    }
    let ya = atan(path_dx(g.z)) + g.turn;
    let rel = xz - vec2f(g.x, g.z);
    // Into the torii's frame at the water line (its lean shifts the pillars there).
    let qx = rel.x * cos(ya) - rel.y * sin(ya);
    let qz = rel.x * sin(ya) + rel.y * cos(ya);
    let shift = g.base * tan(g.lean);
    for (var k = -1.0; k <= 1.0; k += 2.0) {
      let px = (2.0 * k) * g.size - shift;
      let gap = length(vec2f(qx - px, qz)) - 0.38 * g.size;
      if (gap < best.x) {
        best = vec3f(gap, atan2(qz, qx - px), i * 2.0 + k + w * 0.33);
      }
    }
  }
  return best;
}

fn pillar_gap(xz: vec2f) -> f32 {
  return pillar_near(xz).x;
}


// The sea's height at xz: long swells from the open sea, as tall as the music was loud at the
// last change; they come in sets, tall in some stretches and low in others, and their crests
// bend, so far off the sea is not one even ripple.
// A swell with a sharp crest and a broad trough, about -0.7 to 1.3.
fn crest(t: f32) -> f32 {
  let s = 0.5 + 0.5 * sin(t);
  return 2.0 * pow(s, 2.5) - 0.7;
}

fn sea_h(xz: vec2f) -> f32 {
  // Still between changes; each change moves the swells a step on their way, so one set leads
  // into the next.
  let g = state.sea_gen;
  let sets = mix(0.3, SWELL_MAX, smoothstep(-0.4, 0.4, broad(xz / 85.0 + vec2f(g * 0.05, g * 0.02))));
  let a = state.sea_amp * sets;
  let bend = 2.2 * broad(xz / 55.0 + vec2f(3.1, 7.7));
  var h = 0.0;
  h += a * 0.6 * crest(dot(xz, vec2f(0.95, 0.31)) * 0.16 - g * 0.9 + bend);
  h += a * 0.3 * crest(dot(xz, vec2f(0.6, -0.8)) * 0.29 - g * 1.4 + 2.0 + 0.6 * bend);
  h += a * 0.12 * sin(dot(xz, vec2f(0.2, 0.98)) * 0.5 - g * 1.9 + 4.0);
  return h;
}

// ---- The water: a low-poly sea ----
// Even triangles, `facet` metres a side, their corners riding sharp-crested swells (as low-poly
// water is made: an even grid moved by its waves, the triangles showing flat). The sea holds
// still; on the beat it changes, the swells moving on a step and each corner a step through its
// own small rise and fall, so one change leads into the next. Every triangle is one plane, so a
// ray meets it in one step: no marching.

const ROOT3: f32 = 1.7320508;

// The water's height at a corner of the triangles.
// A cheap, broad unevenness, about -0.7..0.7 over a few units of p: crossed sines at odd angles
// (the sea asks for it at every corner it looks at, and only the broad shapes of noise show).
fn broad(p: vec2f) -> f32 {
  return 0.75 * sin(dot(p, vec2f(1.7, 0.6)) + 0.8 * sin(dot(p, vec2f(-0.5, 1.3)) + 1.1)) * sin(dot(p, vec2f(0.4, -1.5)) + 2.3);
}

// Choppy in some patches and smooth in others.
const SWELL_MAX: f32 = 1.5;
const ROUGH_MAX: f32 = 1.6;

fn corner_h(v: vec2f) -> f32 {
  let g = state.sea_gen;
  let cycle = cos(g * 0.8 + hash(v * 0.517 + 3.0) * 6.2832);
  let chop = mix(0.4, ROUGH_MAX, smoothstep(-0.35, 0.35, broad(v / 37.0 + vec2f(9.0, g * 0.04))));
  let rough = (hash(v * 0.731) - 0.5 + 0.35 * cycle) * params.facet_rough * chop * (0.6 + 0.4 * min(state.sea_amp, 2.0));
  return sea_h(v) + rough;
}

// A triangle of the water: its normal, its middle, a number of its own (0..1) and its size.
struct Facet {
  n: vec3f,
  centre: vec3f,
  id: f32,
  size: f32,
}

fn cross2(a: vec2f, b: vec2f) -> f32 {
  return a.x * b.y - a.y * b.x;
}

// Where corner k of the grid lies: rows 60 degrees apart.
fn corner_at(k: vec2f) -> vec2f {
  let s = params.facet;
  return vec2f(s * (k.x + 0.5 * k.y), s * k.y * ROOT3 * 0.5);
}

// Triangle `which` of a cell of the grid (in its skewed coordinates, where each cell is a unit
// square cut along u + v = 1): 0 the one below the cut, 1 the one above.
fn facet_of(cell: vec2f, which: f32) -> Facet {
  return facet_from(corner_p(select(cell, cell + vec2f(1.0, 1.0), which > 0.5)), corner_p(cell + vec2f(1.0, 0.0)), corner_p(cell + vec2f(0.0, 1.0)), cell, which);
}

// Corner k of the grid, at the water's height there.
fn corner_p(k: vec2f) -> vec3f {
  let a = corner_at(k);
  return vec3f(a.x, corner_h(a), a.y);
}

// The triangle with corners pa, pb, pc (pb and pc on the cell's cut).
fn facet_from(pa: vec3f, pb: vec3f, pc: vec3f, cell: vec2f, which: f32) -> Facet {
  let s = params.facet;
  var n = normalize(cross(pc - pa, pb - pa));
  if (n.y < 0.0) {
    n = -n;
  }
  var fa: Facet;
  fa.n = n;
  fa.centre = (pa + pb + pc) / 3.0;
  fa.id = hash(cell * 1.31 + vec2f(which * 7.7, 0.0));
  fa.size = s;
  return fa;
}

// Where xz lies in the grid's skewed coordinates.
fn grid_uv(xz: vec2f) -> vec2f {
  let v = xz.y / (params.facet * ROOT3 * 0.5);
  return vec2f(xz.x / params.facet - 0.5 * v, v);
}

// The triangle above or below xz.
fn facet_at(xz: vec2f) -> Facet {
  let uv = grid_uv(xz);
  let cell = floor(uv);
  let f = uv - cell;
  return facet_of(cell, select(0.0, 1.0, f.x + f.y > 1.0));
}

// How high and how low the water can stand now (see sea_h and corner_h).
fn sea_top() -> f32 {
  return state.sea_amp * 1.3 * SWELL_MAX + params.facet_rough * 1.2 * ROUGH_MAX + 0.02;
}

fn sea_bottom() -> f32 {
  return -(state.sea_amp * 0.76 * SWELL_MAX + params.facet_rough * 1.2 * ROUGH_MAX) - 0.02;
}

// Where a ray first meets the water, and the triangle there: walking the grid's cells under the
// ray in order, from where it comes down to the highest a wave can stand to where it passes the
// lowest, and taking the first triangle whose plane it meets inside the triangle (the water is one
// unbroken surface, so that is where it goes in). A triangle facing the camera is met before the
// ray reaches the level sea, and only a walk finds it. Up to WALK cells, and no further than
// `t_stop` (where the triangles have blended into the swell, and which one it is no longer
// shows): t is 1e9 if the ray is still above the water by then.
const WALK: i32 = 16;

// Quality (frame.quality, 1 is the full picture), all of the sea's knobs: the cells a ray walks
// over the waves (walk_limit, below), 16 at 1 down to 7 at 0; torii a little bigger on the screen
// than the update draws plainly are drawn plainly too below 0.75 (`plain`); and the clouds lose
// octaves, 5 at 0.875 and above, then 4, 3 and 2 from 0.25 down (the sky). The torii's lanterns,
// the spirits and the colours stay at every level.

// How many cells a ray walks at the current quality. Quality 1 is WALK (16, today's picture);
// 0.75 walks 14, 0.5 walks 11, 0.25 walks 9 and 0 walks 7. A ray that runs out of cells without
// meeting a triangle takes the smooth swell instead, so lower levels only lose the far facets'
// edges; the torii, the spirits and the colours are not touched.
fn walk_limit() -> i32 {
  return i32(ceil(f32(WALK) * (0.4 + 0.6 * clamp(frame.quality, 0.0, 1.0))));
}

struct SeaHit {
  t: f32,
  fac: Facet,
}

fn sea_hit(o: vec3f, d: vec3f, t_stop: f32) -> SeaHit {
  var h: SeaHit;
  h.t = 1e9;
  let t0 = max((o.y - sea_top()) / -d.y, 0.0);
  let t1 = min((o.y - sea_bottom()) / -d.y, t_stop);
  if (t0 > t1) {
    return h;
  }
  let uv0 = grid_uv(o.xz + d.xz * t0);
  let dv = d.z / (params.facet * ROOT3 * 0.5);
  let duv = vec2f(d.x / params.facet - 0.5 * dv, dv);
  var cell = floor(uv0);
  let step = sign(duv);
  let still = abs(duv) < vec2f(1e-7);
  let inv = 1.0 / select(duv, vec2f(1.0), still);
  // The t at which the ray next crosses a line of cells, each way, and how far apart they are.
  var next = select((cell + max(step, vec2f(0.0)) - uv0) * inv + t0, vec2f(1e9), still);
  let gap = select(abs(inv), vec2f(1e9), still);
  let walk = walk_limit();
  for (var k = 0; k < walk; k++) {
    // The cell's four corners, the two triangles sharing the two on the cut.
    let c00 = corner_p(cell);
    let c10 = corner_p(cell + vec2f(1.0, 0.0));
    let c01 = corner_p(cell + vec2f(0.0, 1.0));
    let c11 = corner_p(cell + vec2f(1.0, 1.0));
    for (var w = 0.0; w < 2.0; w += 1.0) {
      let f = facet_from(select(c00, c11, w > 0.5), c10, c01, cell, w);
      let dn = dot(d, f.n);
      if (dn < -1e-6) {
        let t = dot(f.centre - o, f.n) / dn;
        let q = grid_uv(o.xz + d.xz * t) - cell;
        let sum = q.x + q.y;
        let inside = all(q >= vec2f(-1e-3)) && all(q <= vec2f(1.001)) && select(sum <= 1.001, sum >= 0.999, w > 0.5);
        if (inside && t >= t0 - 0.01 && t < h.t) {
          h.t = t;
          h.fac = f;
        }
      }
    }
    if (h.t < 1e8 || min(next.x, next.y) > t1) {
      break;
    }
    if (next.x < next.y) {
      cell.x += step.x;
      next.x += gap.x;
    } else {
      cell.y += step.y;
      next.y += gap.y;
    }
  }
  return h;
}

// The swell's own smooth normal, which the far facets blend into, its slope taken across `e`
// metres either way: as wide as a pixel covers, swells too short to see there average away
// instead of striping the far sea.
fn swell_normal(xz: vec2f, e: f32) -> vec3f {
  let hx = sea_h(xz + vec2f(e, 0.0)) - sea_h(xz - vec2f(e, 0.0));
  let hz = sea_h(xz + vec2f(0.0, e)) - sea_h(xz - vec2f(0.0, e));
  return normalize(vec3f(-hx / (2.0 * e), 1.0, -hz / (2.0 * e)));
}

// ---- Light ----

struct Light {
  sun: vec3f,
  sun_colour: vec3f,
  zenith: vec3f,
  horizon: vec3f,
  ambient: vec3f,
  clouds: f32,
  // How much it is night (the lanterns lit, the stars out), and whether the sun or moon shows.
  night: f32,
  disc: f32,
}

// The four lights (the `[[look]]`s of scene.toml, in this order), one for each kind of section:
// 0 a clear day, the sun ahead and up a little, warm light and cool shade;
// 1 a low sun ahead, the torii dark against a bright hazy sky and a path of light on the water;
// 2 the blue hour, the sun just gone: a deep blue sky, a warm glow low on the horizon ahead, a
//   dim warm light from there, and the lanterns coming on;
// 3 a moonlit night.
fn light_of(m: f32) -> Light {
  var l: Light;
  if (m < 0.5) {
    l.sun = normalize(vec3f(-0.4, 0.32, 1.0));
    l.sun_colour = vec3f(1.0, 0.95, 0.85) * 3.0;
    l.zenith = vec3f(0.2, 0.4, 0.78);
    l.horizon = vec3f(0.7, 0.8, 0.9);
    l.ambient = vec3f(0.26, 0.34, 0.5);
    l.clouds = 0.4;
    l.night = 0.0;
    l.disc = 1.0;
  } else if (m < 1.5) {
    l.sun = normalize(vec3f(0.08, 0.06, 1.0));
    l.sun_colour = vec3f(1.0, 0.62, 0.36) * 3.6;
    l.zenith = vec3f(0.2, 0.22, 0.38);
    l.horizon = vec3f(0.95, 0.66, 0.46);
    l.ambient = vec3f(0.3, 0.26, 0.3);
    l.clouds = 0.5;
    l.night = 0.0;
    l.disc = 1.0;
  } else if (m < 2.5) {
    l.sun = normalize(vec3f(0.35, 0.1, 1.0));
    l.sun_colour = vec3f(1.0, 0.55, 0.42) * 1.1;
    l.zenith = vec3f(0.025, 0.05, 0.14);
    l.horizon = vec3f(0.42, 0.3, 0.38);
    l.ambient = vec3f(0.12, 0.16, 0.3);
    l.clouds = 0.25;
    l.night = 0.6;
    l.disc = 0.0;
  } else {
    l.sun = normalize(vec3f(0.35, 0.42, 1.0));
    l.sun_colour = vec3f(0.55, 0.65, 0.9) * 0.55;
    l.zenith = vec3f(0.006, 0.01, 0.025);
    l.horizon = vec3f(0.03, 0.04, 0.07);
    l.ambient = vec3f(0.05, 0.065, 0.1);
    l.clouds = 0.35;
    l.night = 1.0;
    l.disc = 1.0;
  }
  return l;
}

// The looks on screen: the one before, the one now and how far the fade is, all the runtime's
// (`look`, see scene.toml's [[look]]s). The `light_mode` parameter pins one and overrides it.
fn looks_now() -> vec3f {
  if (params.light_mode > 0u) {
    let pinned = f32(params.light_mode - 1u);
    return vec3f(pinned, pinned, 1.0);
  }
  return vec3f(f32(look.prev), f32(look.to), look.mix);
}

// The light now: the look before fading into the look now (the runtime's mix is eased already).
fn light_now() -> Light {
  let now = looks_now();
  var l = light_of(now.y);
  if (now.z < 1.0) {
    let a = light_of(now.x);
    l.sun = normalize(mix(a.sun, l.sun, now.z));
    l.sun_colour = mix(a.sun_colour, l.sun_colour, now.z);
    l.zenith = mix(a.zenith, l.zenith, now.z);
    l.horizon = mix(a.horizon, l.horizon, now.z);
    l.ambient = mix(a.ambient, l.ambient, now.z);
    l.clouds = mix(a.clouds, l.clouds, now.z);
    l.night = mix(a.night, l.night, now.z);
    l.disc = mix(a.disc, l.disc, now.z);
  }
  return l;
}

fn sky(d: vec3f, l: Light) -> vec3f {
  let up = max(d.y, 0.0);
  var c = mix(l.horizon, l.zenith, pow(up, 0.55));
  // Below the horizon (seen past the far sea) the haze.
  c = mix(c, l.horizon * 0.8, smoothstep(0.0, -0.05, d.y));
  let s = max(dot(d, l.sun), 0.0);
  // The sun (or the moon), its glow and a wide warm halo.
  c += l.sun_colour * (smoothstep(0.9993, 0.9997, s) * 6.0 * l.disc + pow(s, 64.0) * 0.35 + pow(s, 6.0) * 0.08);
  // Clouds on a layer far above.
  if (d.y > 0.01) {
    let uv = d.xz / (d.y + 0.08) * 0.7 + vec2f(frame.time * 0.004, 0.0);
    // Below full quality the finest of the clouds' detail goes, an octave a step (to two).
    let n = rv_fbm(uv * 1.3, 5 - i32(clamp(round((1.0 - frame.quality) * 4.0), 0.0, 3.0)));
    let cover = smoothstep(0.35 - 0.55 * l.clouds, 0.6, n + 0.25);
    let lit = mix(l.horizon * 0.9, l.sun_colour * 0.35 + l.horizon * 0.5, pow(s, 3.0));
    c = mix(c, lit * (0.75 + 0.35 * n), cover * smoothstep(0.01, 0.15, d.y) * min(l.clouds + 0.3, 1.0));
  }
  // Stars at night.
  if (l.night > 0.0 && d.y > 0.0) {
    let g = floor(d.xz / (d.y + 0.3) * 180.0);
    let star = step(0.997, hash(g)) * l.night * smoothstep(0.05, 0.3, d.y);
    c += vec3f(0.8, 0.85, 1.0) * star * 0.6;
  }
  // Rocks and islands on the horizon, far off in the haze: a silhouette by bearing.
  if (d.y > -0.002 && d.y < 0.06) {
    let az = atan2(d.x, d.z);
    let big = max(rv_noise(vec2f(az * 3.0, 0.5)) - 0.12, 0.0);
    let small = max(rv_noise(vec2f(az * 11.0, 2.5)) - 0.3, 0.0);
    let h = big * 0.09 * params.stacks * 2.0 + small * 0.05 * params.stacks * 2.0;
    let edge = h + 0.004 * rv_noise(vec2f(az * 90.0, 4.0));
    if (d.y < edge) {
      let rock = vec3f(0.05, 0.05, 0.055) * (l.ambient + l.sun_colour * 0.15);
      c = mix(rock, l.horizon, 0.55);
    }
  }
  return c;
}

// How brightly the lanterns of the line's torii glow: dimmer by day, full at night; from dim in a
// quiet passage to bright in a loud one (`energy`, the moment's energy spread over its range), and
// each torii's with a band of its own (bass, mids or treble), so the lanterns down the line
// answer different parts of the music.
fn lantern_glow(i: f32, l: Light) -> f32 {
  let band = clamp(band(u32(abs(i)) % 3u) / 1.5, 0.0, 1.0);
  return params.lanterns * mix(1.0, 1.6, l.night) * mix(0.3, 1.8, input.energy) * (0.6 + 0.8 * band);
}

const LANTERN: vec3f = vec3f(1.0, 0.52, 0.2);

// The spirits' colour: the pale blue-green of fox fire, a touch warmer by day, bluer as night
// comes.
fn spirit_of(m: f32) -> vec3f {
  if (m < 0.5) {
    return vec3f(0.72, 0.97, 0.95);
  }
  if (m < 1.5) {
    return vec3f(0.78, 0.96, 0.9);
  }
  if (m < 2.5) {
    return vec3f(0.6, 0.94, 1.0);
  }
  return vec3f(0.5, 0.88, 1.0);
}

// The spirits in the theme's colour: each light's own lightness (fading with the look), the
// theme's hue.
fn spirit_colour() -> vec3f {
  let now = looks_now();
  return themed(mix(spirit_of(now.x), spirit_of(now.y), now.z), 0.14);
}
