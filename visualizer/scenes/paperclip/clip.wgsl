// The dancers, shared by the passes: each clip is its wire as a chain of capsules round the posed
// centre line, two eye balls and two brows, met exactly by a ray (nothing is marched). The lead
// stands at the origin; the chorus stands one to a floor cell in rows behind it, walked cell by
// cell along the ray.

// Where a ray first meets a capsule from a to b of radius r, or -1 (Inigo Quilez).
fn capsule_hit(ro: vec3f, rd: vec3f, a: vec3f, b: vec3f, r: f32) -> f32 {
  let ba = b - a;
  let oa = ro - a;
  let baba = dot(ba, ba);
  let bard = dot(ba, rd);
  let baoa = dot(ba, oa);
  let rdoa = dot(rd, oa);
  let oaoa = dot(oa, oa);
  let qa = baba - bard * bard;
  let qb = baba * rdoa - baoa * bard;
  let qc = baba * oaoa - baoa * baoa - r * r * baba;
  let h = qb * qb - qa * qc;
  if (h >= 0.0) {
    let t = (-qb - sqrt(h)) / qa;
    let y = baoa + t * bard;
    if (y > 0.0 && y < baba) {
      return t;
    }
    let oc = select(ro - b, oa, y <= 0.0);
    let b2 = dot(rd, oc);
    let h2 = b2 * b2 - dot(oc, oc) + r * r;
    if (h2 > 0.0) {
      return -b2 - sqrt(h2);
    }
  }
  return -1.0;
}

fn sphere_hit(ro: vec3f, rd: vec3f, c: vec3f, r: f32) -> f32 {
  let oc = ro - c;
  let b = dot(oc, rd);
  let h = b * b - dot(oc, oc) + r * r;
  if (h < 0.0) {
    return -1.0;
  }
  return -b - sqrt(h);
}

// What a ray met: 0 nothing, 1 the wire, 2 an eye, 3 a brow, 4 the floor. For a dancer: its
// group, where it stands, and which eye.
struct Hit {
  t: f32,
  n: vec3f,
  kind: u32,
  group: u32,
  at: vec3f,
  eye: u32,
}

fn point(g: u32, i: u32) -> vec3f {
  return state.pts[g * POINTS + i].xyz;
}

// The dancer of group g standing at `at`, if the ray meets it nearer than hit.t.
fn dancer(ro_world: vec3f, rd: vec3f, g: u32, at: vec3f, hit: ptr<function, Hit>, detail: bool) {
  let ro = ro_world - at;
  let bound = state.bound[g];
  if (sphere_hit(ro, rd, bound.xyz, bound.w) < 0.0 && length(ro - bound.xyz) > bound.w) {
    return;
  }
  for (var i = 0u; i + 1u < WIRE; i++) {
    let a = point(g, i);
    let b = point(g, i + 1u);
    let t = capsule_hit(ro, rd, a, b, WIRE_R);
    if (t > 0.0 && t < (*hit).t) {
      let p = ro + rd * t;
      let ba = b - a;
      let h = clamp(dot(p - a, ba) / dot(ba, ba), 0.0, 1.0);
      (*hit).t = t;
      (*hit).n = normalize(p - a - ba * h);
      (*hit).kind = 1u;
      (*hit).group = g;
      (*hit).at = at;
    }
  }
  for (var e = 0u; e < 2u; e++) {
    let c = point(g, WIRE + e);
    let t = sphere_hit(ro, rd, c, EYE_R);
    if (t > 0.0 && t < (*hit).t) {
      (*hit).t = t;
      (*hit).n = normalize(ro + rd * t - c);
      (*hit).kind = 2u;
      (*hit).group = g;
      (*hit).at = at;
      (*hit).eye = e;
    }
    if (detail) {
      let a = point(g, WIRE + 2u + e * 2u);
      let b = point(g, WIRE + 3u + e * 2u);
      let tb = capsule_hit(ro, rd, a, b, BROW_R);
      if (tb > 0.0 && tb < (*hit).t) {
        (*hit).t = tb;
        (*hit).n = normalize(ro + rd * tb - 0.5 * (a + b));
        (*hit).kind = 3u;
        (*hit).group = g;
        (*hit).at = at;
      }
    }
  }
}

// The chorus dancer of floor cell c, if there is one: in the rows behind the lead.
fn in_chorus(c: vec2f) -> bool {
  return (c.y + 0.5) * CELL >= FIRST_ROW && params.chorus > 0.0;
}

fn cell_at(c: vec2f) -> vec3f {
  return vec3f((c.x + 0.5) * CELL, 0.0, (c.y + 0.5) * CELL);
}

// The first thing along the ray: the lead, the chorus (walking the floor's cells while the ray is
// low enough to meet one), else the floor.
fn trace(ro: vec3f, rd: vec3f, steps: i32) -> Hit {
  var hit: Hit;
  hit.t = 1e9;
  hit.kind = 0u;
  dancer(ro, rd, 0u, vec3f(0.0), &hit, true);

  // The layer the dancers stand in.
  // (A backflip goes higher than a clip stands.)
  let top = 2.3;
  var t0 = 0.0;
  var t1 = 1e9;
  if (abs(rd.y) > 1e-5) {
    let ta = (top - ro.y) / rd.y;
    let tb = -ro.y / rd.y;
    t0 = max(min(ta, tb), 0.0);
    t1 = max(ta, tb);
  } else if (ro.y > top || ro.y < 0.0) {
    t1 = -1.0;
  }
  if (t1 > t0) {
    let d = vec2f(select(rd.x, 1e-6, abs(rd.x) < 1e-6), select(rd.z, 1e-6, abs(rd.z) < 1e-6));
    let p = ro.xz + d * (t0 + 1e-4);
    var c = floor(p / CELL);
    let stp = sign(d);
    let delta = abs(CELL / d);
    var next = ((c + max(stp, vec2f(0.0))) * CELL - ro.xz) / d;
    for (var k = 0; k < steps; k++) {
      if (in_chorus(c)) {
        dancer(ro, rd, group_of(c), cell_at(c), &hit, false);
      }
      let tn = min(next.x, next.y);
      if (hit.t < tn || tn > t1) {
        break;
      }
      if (next.x < next.y) {
        c.x += stp.x;
        next.x += delta.x;
      } else {
        c.y += stp.y;
        next.y += delta.y;
      }
    }
  }
  if (hit.kind == 0u && rd.y < 0.0) {
    hit.t = -ro.y / rd.y;
    hit.n = vec3f(0.0, 1.0, 0.0);
    hit.kind = 4u;
  }
  return hit;
}
