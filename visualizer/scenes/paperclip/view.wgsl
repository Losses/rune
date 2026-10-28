// The dancers as the camera sees them, drawn small (the pixel pass blows them up and dithers
// them): chrome wire lit from high on the left and reflecting a bright sky over a dark floor, eye
// balls drawn with the face of the moment, papers under their feet with their soft shadows on
// them, a dark tiled floor fading into the dark.

struct Ray {
  o: vec3f,
  d: vec3f,
}

fn camera_ray(px: Pixel) -> Ray {
  let f = normalize(state.aim - state.cam);
  let right0 = normalize(cross(vec3f(0.0, 1.0, 0.0), f));
  let up0 = cross(f, right0);
  // The shot's dutch angle: the picture's axes turned about the way the camera looks.
  let c = cos(state.roll);
  let s = sin(state.roll);
  let right = right0 * c + up0 * s;
  let up = up0 * c - right0 * s;
  var ray: Ray;
  ray.o = state.cam;
  ray.d = normalize(f + (right * px.p.x + up * px.p.y) * 2.0 * state.lens);
  return ray;
}

fn light() -> vec3f {
  return normalize(LIGHT);
}

// The sky the chrome reflects: bright above, the dark floor below.
fn reflected(r: vec3f) -> vec3f {
  let sky = mix(vec3f(0.35, 0.37, 0.48), vec3f(1.0, 1.0, 1.05), smoothstep(0.0, 0.8, r.y));
  return mix(vec3f(0.02, 0.022, 0.03), sky, smoothstep(-0.15, 0.1, r.y));
}

fn chrome(n: vec3f, d: vec3f) -> vec3f {
  let l = light();
  let r = reflect(d, n);
  let diffuse = max(dot(n, l), 0.0);
  let spec = pow(max(dot(r, l), 0.0), 50.0);
  return params.metal.rgb * (0.12 + 0.6 * diffuse) + reflected(r) * 0.35 + vec3f(1.6) * spec;
}

// ---- Faces ----

fn segment(p: vec2f, a: vec2f, b: vec2f) -> f32 {
  return rv_sd_segment(p, a, b);
}

// A five-pointed star of radius r (Inigo Quilez).
fn star(p0: vec2f, r: f32) -> f32 {
  let k1 = vec2f(0.809016994375, -0.587785252292);
  let k2 = vec2f(-k1.x, k1.y);
  var p = vec2f(abs(p0.x), p0.y);
  p -= 2.0 * max(dot(k1, p), 0.0) * k1;
  p -= 2.0 * max(dot(k2, p), 0.0) * k2;
  p = vec2f(abs(p.x), p.y - r);
  let ba = 0.45 * vec2f(-k1.y, k1.x) - vec2f(0.0, 1.0);
  let h = clamp(dot(p, ba) / dot(ba, ba), 0.0, r);
  return length(p - ba * h) * sign(p.y * ba.x - p.x * ba.y);
}

// The face on eye ball `e` (0 left, 1 right) at g, coordinates across the ball's front (-1..1,
// y up): the ink (1 drawn) and the lid (1 where the ball is covered by the closed lid).
fn face_ink(face: u32, e: u32, g0: vec2f, look: vec2f) -> vec2f {
  // Strokes bold enough to read in big pixels.
  let w = 0.17;
  let mirror = select(-1.0, 1.0, e == 0u);
  var g = g0;
  switch face {
    case 1u: {
      // >_< : the left eye a >, the right a <.
      g.x *= mirror;
      let d = min(segment(g, vec2f(-0.5, 0.42), vec2f(0.38, 0.0)), segment(g, vec2f(0.38, 0.0), vec2f(-0.5, -0.42)));
      return vec2f(1.0 - smoothstep(w * 0.7, w, d), 0.0);
    }
    case 2u: {
      // ^ ^ : happy arcs.
      let d = abs(length(g - vec2f(0.0, -0.25)) - 0.55);
      return vec2f((1.0 - smoothstep(w * 0.7, w, d)) * step(-0.25, g.y), 0.0);
    }
    case 3u: {
      // @ @ : a dizzy spiral of one and a half turns, turning slowly: one line the same width all
      // along, with round ends, from just off the centre out to 0.75.
      let q = rv_rotate(g, frame.time * 2.0 * mirror);
      let pitch = 0.375;
      let a = atan2(q.y, q.x) / 6.28318;
      let k = length(q) / pitch - a;
      // How many turns out along the line the nearest stretch of it is (it runs from 0.5 to 2), and
      // how far off it this point is; past an end, how far from that end.
      let turns = floor(k) + 0.5 + a;
      var d = select(1.0, abs(fract(k) - 0.5) * pitch, turns >= 0.5 && turns <= 2.0);
      let inner = 0.5 * pitch * vec2f(1.0, 0.0);
      let outer = 2.0 * pitch * vec2f(-1.0, 0.0);
      d = min(d, min(length(q - inner), length(q - outer)));
      let stroke = 0.11;
      return vec2f(1.0 - smoothstep(stroke * 0.7, stroke, d), 0.0);
    }
    case 4u: {
      // Wide eyed: a small pupil.
      return vec2f(1.0 - smoothstep(0.2, 0.24, length(g - look * 0.3)), 0.0);
    }
    case 5u: {
      // – – : drowsy, the lid half down.
      let d = abs(g.y - 0.05);
      return vec2f(1.0 - smoothstep(w * 0.5, w * 0.8, d) * step(abs(g.x), 0.85), step(0.05, g.y));
    }
    case 6u: {
      // Stars in the eyes.
      return vec2f(1.0 - smoothstep(-0.01, 0.03, star(g, 0.62)), 0.0);
    }
    case 7u: {
      // A blink: the lid shut.
      let d = abs(g.y + 0.1);
      return vec2f(1.0 - smoothstep(w * 0.5, w * 0.8, d), step(-0.1, g.y));
    }
    default: {
      return vec2f(1.0 - smoothstep(0.38, 0.42, length(g - look * 0.45)), 0.0);
    }
  }
}

fn shade_eye(p: vec3f, n: vec3f, d: vec3f, h: Hit) -> vec3f {
  let g = h.group;
  let centre = point(g, WIRE + h.eye) + h.at;
  let look = vec2f(state.pts[g * POINTS + WIRE].w, state.pts[g * POINTS + WIRE + 1u].w);
  let face = u32(state.pts[g * POINTS + WIRE + 2u].w + 0.5);
  // The face is on the front of the head, turned towards the camera by at most GLANCE (so it reads
  // from in front and is seen side on from the side, and from behind not at all).
  let base = g * POINTS;
  let fwd = vec3f(state.pts[base].w, state.pts[base + 1u].w, state.pts[base + 2u].w);
  let head_up = vec3f(state.pts[base + 3u].w, state.pts[base + 4u].w, state.pts[base + 5u].w);
  let to_cam = normalize(state.cam - centre);
  let towards = dot(fwd, to_cam);
  var f = to_cam;
  if (towards < cos(GLANCE)) {
    let across = normalize(to_cam - fwd * towards + head_up * 1e-3);
    f = fwd * cos(GLANCE) + across * sin(GLANCE);
  }
  let right = normalize(cross(f, head_up));
  let up = cross(right, f);
  let front = dot(n, f);
  let ink = face_ink(face, h.eye, vec2f(dot(n, right), dot(n, up)) / 0.8, look) * step(0.0, front);
  let l = light();
  let r = reflect(d, n);
  let diffuse = max(dot(n, l), 0.0);
  var c = vec3f(0.92, 0.92, 0.9) * (0.25 + 0.75 * diffuse);
  c = mix(c, chrome(n, d) * 0.6, ink.y);
  c = mix(c, vec3f(0.01), ink.x);
  return c + vec3f(1.2) * pow(max(dot(r, l), 0.0), 80.0);
}

// ---- The floor ----

// The paper under a dancer standing at `at`, its own size and turn: whether p is on it.
fn on_paper(p: vec2f, at: vec2f, half: vec2f, turn: f32) -> bool {
  let q = rv_rotate(p - at, turn);
  return all(abs(q) < half);
}

// ---- Shadows ----
//
// The shadow of a dancer is its wire cast along the light onto the floor as chains of capsules (the
// segments between the posed points), the eye balls as discs and the brows as thin capsules.
// Measured where the floor is squashed along the light's side so that a cast sphere is a circle
// (`squash`). The shadow's edge is as soft as the casting point is high over the floor (the light
// is not a point): sharp where the wire touches the paper, soft at the head, and a clip in the
// air is lifted off the floor and softer.
const PEN_FOOT: f32 = 0.012;
const PEN_PER_HEIGHT: f32 = 0.08;

// Floor positions squashed along the light's floor direction by the light's slant: a sphere's
// shadow, an ellipse stretched by that, is a circle in them.
fn squash(v: vec2f) -> vec2f {
  let l = LIGHT;
  let slant = l.y / length(l);
  let u = normalize(l.xz);
  return v - u * (dot(v, u) * (1.0 - slant));
}

// Where a point q, cast along the light onto the floor, falls (squashed).
fn cast_down(q: vec3f) -> vec2f {
  return squash(q.xz - LIGHT.xz * (q.y / LIGHT.y));
}

fn penumbra(height: f32) -> f32 {
  return PEN_FOOT + PEN_PER_HEIGHT * max(height, 0.0);
}

// How far inside the shadow of the disc or capsule from a to b (radius r) the floor point at `p`
// (squashed) is, in the penumbra's widths: below -1 the whole shadow, above 1 none. The edge sits
// half a width outside r, so that a soft shadow keeps its core.
fn edge(p: vec2f, a: vec3f, b: vec3f, r: f32) -> f32 {
  let sa = cast_down(a);
  let ba = cast_down(b) - sa;
  let pa = p - sa;
  let h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-8), 0.0, 1.0);
  let w = penumbra(mix(a.y, b.y, h));
  return (length(pa - ba * h) - r) / w - 0.5;
}

// How much of the light dancer g standing at `at` takes from floor point p (squashed, `pf`): 0
// none, 1 all. A dancer whose cast bound is farther than its widest penumbra costs this one test.
fn shade_by(pf: vec2f, g: u32, at: vec3f) -> f32 {
  let bound = state.bound[g];
  let reach = bound.w + EYE_R + 1.5 * penumbra(bound.y + bound.w);
  if (length(pf - cast_down(bound.xyz + at)) > reach) {
    return 0.0;
  }
  var near = 1e9;
  let stride = select(4u, 2u, frame.quality > 0.5);
  for (var i = 0u; i + 1u < WIRE; i += stride) {
    near = min(near, edge(pf, point(g, i) + at, point(g, min(i + stride, WIRE - 1u)) + at, WIRE_R));
  }
  for (var e = 0u; e < 2u; e++) {
    let c = point(g, WIRE + e) + at;
    near = min(near, edge(pf, c, c, EYE_R));
    if (frame.quality > 0.5) {
      near = min(near, edge(pf, point(g, WIRE + 2u + e * 2u) + at, point(g, WIRE + 3u + e * 2u) + at, BROW_R));
    }
  }
  return 1.0 - smoothstep(-1.0, 1.0, near);
}

// The light left at floor point p in cell c: past the lead and past the chorus dancers of the cell
// and of the three next to it up the light (their shadows fall across the cell's edge).
fn lit_at(p: vec3f, c: vec2f) -> f32 {
  let pf = squash(p.xz);
  var taken = shade_by(pf, 0u, vec3f(0.0));
  let up = sign(LIGHT.xz);
  for (var k = 0u; k < 4u; k++) {
    let cc = c + vec2f(f32(k & 1u), f32(k >> 1u)) * up;
    if (in_chorus(cc)) {
      taken = max(taken, shade_by(pf, group_of(cc), cell_at(cc)));
    }
  }
  return 1.0 - params.shadow * taken;
}

fn shade_floor(p: vec3f, d: vec3f, t: f32) -> vec3f {
  let l = light();
  let c = floor(p.xz / CELL);
  var lit = max(l.y, 0.0);
  var albedo = vec3f(0.03, 0.032, 0.045);
  // Large dark tiles.
  let tile = floor(p.xz / 0.65);
  albedo *= select(0.75, 1.0, fract((tile.x + tile.y) * 0.5) > 0.25);
  // The papers: a big one under the lead, a small one under each of the chorus.
  if (on_paper(p.xz, vec2f(0.05, 0.05), vec2f(0.62, 0.48), 0.5)) {
    albedo = state.paper;
  } else if (in_chorus(c) && on_paper(p.xz, cell_at(c).xz, vec2f(0.4, 0.3), hash(c + 0.7) * 3.14)) {
    albedo = state.paper * 0.9;
  }
  lit *= lit_at(p, c);
  let col = albedo * (0.12 + 1.04 * lit);
  // Far off, the floor is lost in the dark.
  return mix(params.background.rgb, col, exp(-max(t - 4.0, 0.0) * params.fade));
}

// Quality (frame.quality, 1 is the full picture). Level 1 walks 28 floor cells per ray and casts
// the shadow from a segment of the wire between every second point, and the brows'. Lower levels
// walk fewer cells (the far rows of the chorus, which fade into the dark, drop out first) and
// from 0.5 down cast it from every fourth point (the shadow's bends turn coarser) and without the
// brows; the lead, the faces and the colours stay.
fn view_steps() -> i32 {
  return i32(round(mix(14.0, 28.0, frame.quality)));
}

fn shade(px: Pixel) -> vec4f {
  let ray = camera_ray(px);
  let h = trace(ray.o, ray.d, view_steps());
  if (h.kind == 0u) {
    return vec4f(params.background.rgb, 1.0);
  }
  let p = ray.o + ray.d * h.t;
  var c: vec3f;
  if (h.kind == 1u) {
    c = chrome(h.n, ray.d);
  } else if (h.kind == 2u) {
    c = shade_eye(p, h.n, ray.d, h);
  } else if (h.kind == 3u) {
    c = vec3f(0.02) + vec3f(0.3) * pow(max(dot(reflect(ray.d, h.n), light()), 0.0), 30.0);
  } else {
    c = shade_floor(p, ray.d, h.t);
  }
  // Dancers far back fade into the dark too.
  if (h.kind != 4u) {
    c = mix(params.background.rgb, c, exp(-max(h.t - 4.0, 0.0) * params.fade));
  }
  return vec4f(c, 1.0);
}
