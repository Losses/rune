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
  let right = normalize(cross(vec3f(0.0, 1.0, 0.0), f));
  let up = cross(f, right);
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
      // @ @ : dizzy spirals, two turns wide apart and bold enough for big pixels, turning slowly.
      let q = rv_rotate(g, frame.time * 2.0 * mirror);
      let r = length(q);
      let a = atan2(q.y, q.x) / 6.28318;
      let k = r / 0.36 - a;
      let d = abs(fract(k) - 0.5) * 0.36;
      return vec2f((1.0 - smoothstep(w * 0.55, w * 0.8, d)) * step(r, 0.8), 0.0);
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

// How much light reaches floor point p past a dancer of group g standing at `at`: the soft
// shadow of its wire, cast along the light.
fn shadow(p: vec3f, g: u32, at: vec3f) -> f32 {
  let l = light();
  var near = 1e9;
  for (var i = 0u; i < WIRE; i += 2u) {
    let q = point(g, i) + at;
    let s = q.xz - l.xz * (q.y / l.y);
    near = min(near, length(p.xz - s));
  }
  return mix(1.0 - params.shadow, 1.0, smoothstep(0.03, 0.22, near));
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
  lit *= shadow(p, 0u, vec3f(0.0));
  if (in_chorus(c)) {
    lit *= shadow(p, group_of(c), cell_at(c));
  }
  let col = albedo * (0.25 + 0.9 * lit);
  // Far off, the floor is lost in the dark.
  return mix(params.background.rgb, col, exp(-max(t - 4.0, 0.0) * params.fade));
}

fn shade(px: Pixel) -> vec4f {
  let ray = camera_ray(px);
  let h = trace(ray.o, ray.d, 28);
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
