// The sphere floating in space, met exactly by a ray (no marching): its rings (flat bands of
// gold-tinted glass with glowing edges) and the core of light; how they look by the
// core's warm light and the faint cool light of the stars; and the space behind, stars and
// nebulae.

// The core's light and the glass's colour, from the palette (see update).
fn core_colour() -> vec3f {
  return state.palette[1].rgb;
}

fn brass_colour() -> vec3f {
  return state.palette[12].rgb;
}
const MISS: vec4f = vec4f(1e9, 0.0, 0.0, 0.0);

// A ray in a band's own frame (z its axis) against the band: between radii r0 and r1, half
// thickness h. Distance and normal in the frame, or MISS.
fn washer_hit(ro: vec3f, rd: vec3f, r1: f32, r0: f32, h: f32) -> vec4f {
  var s0 = -1e9;
  var s1 = 1e9;
  if (abs(rd.z) < 1e-8) {
    if (abs(ro.z) > h) {
      return MISS;
    }
  } else {
    let a = (-h - ro.z) / rd.z;
    let b = (h - ro.z) / rd.z;
    s0 = min(a, b);
    s1 = max(a, b);
  }
  let aa = dot(rd.xy, rd.xy);
  if (aa < 1e-12) {
    let r = length(ro.xy);
    if (r > r1 || r < r0 || s0 < 0.0) {
      return MISS;
    }
    return vec4f(s0, 0.0, 0.0, -sign(rd.z));
  }
  let bb = dot(ro.xy, rd.xy);
  let d1 = bb * bb - aa * (dot(ro.xy, ro.xy) - r1 * r1);
  if (d1 < 0.0) {
    return MISS;
  }
  let q1 = sqrt(d1);
  let c0 = (-bb - q1) / aa;
  let c1 = (-bb + q1) / aa;
  var t0 = max(s0, c0);
  let t1 = min(s1, c1);
  if (t0 > t1 || t1 < 0.0) {
    return MISS;
  }
  var n = vec3f(0.0, 0.0, -sign(rd.z));
  if (c0 > s0) {
    n = vec3f(normalize((ro + rd * c0).xy), 0.0);
  }
  // Outside the hole: if the ray enters the slab within the hole, the band starts where it
  // leaves the hole.
  let d0 = bb * bb - aa * (dot(ro.xy, ro.xy) - r0 * r0);
  if (d0 > 0.0 && r0 > 0.0) {
    let q0 = sqrt(d0);
    let i0 = (-bb - q0) / aa;
    let i1 = (-bb + q0) / aa;
    if (t0 > i0 && t0 < i1) {
      if (i1 > t1) {
        return MISS;
      }
      t0 = i1;
      n = vec3f(-normalize((ro + rd * i1).xy), 0.0);
    }
  }
  if (t0 < 0.0) {
    return MISS;
  }
  return vec4f(t0, n);
}

// Ring i: the ray taken into its frame. Distance and normal in the world.
fn ring_hit(i: u32, o: vec3f, d: vec3f) -> vec4f {
  let x = state.ring_x[i];
  let y = state.ring_y[i];
  let z = state.ring_z[i];
  let rel = o - state.centre;
  let ro = vec3f(dot(rel, x.xyz), dot(rel, y.xyz), dot(rel, z.xyz));
  let rd = vec3f(dot(d, x.xyz), dot(d, y.xyz), dot(d, z.xyz));
  let h = washer_hit(ro, rd, x.w, x.w - y.w, z.w);
  if (h.x > 1e8) {
    return MISS;
  }
  return vec4f(h.x, x.xyz * h.y + y.xyz * h.z + z.xyz * h.w);
}

fn sphere_hit(o: vec3f, d: vec3f, c: vec3f, r: f32) -> vec4f {
  let oc = o - c;
  let b = dot(oc, d);
  let disc = b * b - dot(oc, oc) + r * r;
  if (disc < 0.0) {
    return MISS;
  }
  let t = -b - sqrt(disc);
  if (t < 0.0) {
    return MISS;
  }
  return vec4f(t, normalize(oc + d * t));
}

// ---- The gem ----

// How far the gem's facets are from its middle: a little more than the plain core, swelling
// with it.
fn gem_radius() -> f32 {
  return core_radius() * 1.15;
}

// A ray against the gem (from outside): where it enters and the facet's normal, or MISS.
fn gem_hit(o: vec3f, d: vec3f) -> vec4f {
  let r = gem_radius();
  // (Its corners stand a little proud of the ball its facets touch.)
  if (sphere_hit(o, d, state.centre, r * 1.12).x > 1e8) {
    return MISS;
  }
  let rel = o - state.centre;
  var enter = -1e9;
  var leave = 1e9;
  var n = vec3f(0.0);
  for (var k = 0u; k < state.gem_count; k++) {
    let f = state.gem[k];
    let den = dot(f.xyz, d);
    let gap = f.w * r - dot(f.xyz, rel);
    if (abs(den) < 1e-6) {
      if (gap < 0.0) {
        return MISS;
      }
      continue;
    }
    let t = gap / den;
    if (den < 0.0) {
      if (t > enter) {
        enter = t;
        n = f.xyz;
      }
    } else {
      leave = min(leave, t);
    }
  }
  if (enter > leave || enter < 0.0) {
    return MISS;
  }
  return vec4f(enter, n);
}

// From p inside the gem (from its centre) along d: how far to the facet it reaches, and that
// facet's outward normal.
fn gem_exit(p: vec3f, d: vec3f) -> vec4f {
  let r = gem_radius();
  var best = vec4f(1e9, 0.0, 1.0, 0.0);
  for (var k = 0u; k < state.gem_count; k++) {
    let f = state.gem[k];
    let den = dot(f.xyz, d);
    if (den > 1e-6) {
      let t = (f.w * r - dot(f.xyz, p)) / den;
      if (t < best.x) {
        best = vec4f(max(t, 0.0), f.xyz);
      }
    }
  }
  return best;
}

// How far p (from the gem's centre), on the facet facing n, is from that facet's nearest edge:
// where each other facet's plane cuts it.
fn gem_edge(p: vec3f, n: vec3f) -> f32 {
  let r = gem_radius();
  var edge = 1e9;
  for (var k = 0u; k < state.gem_count; k++) {
    let f = state.gem[k];
    let across = 1.0 - dot(f.xyz, n) * dot(f.xyz, n);
    if (across > 1e-4) {
      edge = min(edge, (f.w * r - dot(f.xyz, p)) / sqrt(across));
    }
  }
  return edge;
}

// ---- Light ----

// The faint light of the stars and nebulae: cool, from one side more than the others.
const STARLIGHT: vec3f = vec3f(-0.55, 0.45, 0.7);

// How bright the core is, and how big: both follow the music (smoothly), and it swells on the
// kick.
fn core_power() -> f32 {
  return params.core * (0.7 + 0.6 * state.moment) * 14.0;
}

fn core_radius() -> f32 {
  return 0.3 * state.core_size;
}

// How much of the core's light reaches p: each ring in the way, being glass, lets some through.
fn core_reaches(p: vec3f) -> f32 {
  let to = state.centre - p;
  let dist = length(to);
  let d = to / dist;
  // Stop short of the core itself.
  let reach = dist - core_radius() - 0.1;
  var through = 1.0;
  for (var i = 0u; i < RINGS; i++) {
    if (ring_hit(i, p, d).x < reach) {
      // (Bronze lets almost none through.)
      through *= select(0.4, 0.06, params.look == 1u);
    }
  }
  return through;
}

// The light falling on p facing n: the core (with its shadows when asked), and the starlight.
fn light_at(p: vec3f, n: vec3f, shadows: bool) -> vec3f {
  let to = state.centre - p;
  let dist2 = dot(to, to);
  let l = to * inverseSqrt(dist2);
  var core = max(dot(n, l), 0.0) * core_power() / (dist2 + 1.0);
  if (shadows && core > 0.0) {
    core *= core_reaches(p);
  }
  let stars = max(dot(n, normalize(STARLIGHT)), 0.0) * 0.08;
  return core_colour() * core + state.palette[9].rgb * (stars + 0.035);
}

// What a shiny surface facing r mirrors: deep space, brighter towards the nebula the starlight
// comes from.
fn env(r: vec3f) -> vec3f {
  let toward = max(dot(r, normalize(STARLIGHT)), 0.0);
  return state.palette[10].rgb + state.palette[11].rgb * pow(toward, 3.0);
}

// The core's own highlight on a shiny surface at p facing n, seen along d, `sharp` how tight.
fn core_glint(p: vec3f, n: vec3f, d: vec3f, sharp: f32) -> vec3f {
  let to = state.centre - p;
  let dist2 = dot(to, to);
  let s = pow(max(dot(reflect(d, n), to * inverseSqrt(dist2)), 0.0), sharp);
  return core_colour() * s * core_power() / (dist2 + 1.0) * sharp * 0.02;
}

// Engraved lines: how much of a line a point at arc distance `off` (metres) from it is in,
// with lines `width` wide and a pixel `foot` wide.
fn groove(off: f32, width: f32, foot: f32) -> f32 {
  let w = max(width, foot);
  return (1.0 - smoothstep(w * 0.5, w * 0.5 + foot, abs(off))) * min(width / w, 1.0);
}

// A ring as one layer over what is behind it: what it shows itself, and how much of what is
// behind shows through it.
struct Layer {
  front: vec3f,
  lets: vec3f,
}

// Ring i at p (world), facing n, seen along d: glass with gold in it, mostly solid (so the rings
// in front plainly cover those behind), clearest face on and more mirror at a slant (as glass
// is), the edges of the band glowing gold-white and its ticks lines of light.
fn ring_layer(i: u32, p: vec3f, n: vec3f, d: vec3f, foot: f32) -> Layer {
  let x = state.ring_x[i];
  let y = state.ring_y[i];
  let z = state.ring_z[i];
  let look = state.ring_look[i];
  let rel = p - state.centre;
  let lz = dot(rel, z.xyz);
  let radius = length(rel - z.xyz * lz);
  // How far to the nearest edge of the face it was met on, in metres.
  var edge = z.w - abs(lz);
  if (abs(dot(n, z.xyz)) > 0.5) {
    edge = min(x.w - radius, radius - (x.w - y.w));
  }
  let rim = exp(-max(edge, 0.0) / max(0.01, foot));
  var lines = 0.0;
  if (look.x > 0.5) {
    // Ticks: every one short, every sixth long, across the band from its outer edge.
    let a = atan2(dot(rel, y.xyz), dot(rel, x.xyz));
    let pos = a / 6.2831853 * look.x;
    let off = (fract(pos + 0.5) - 0.5) * 6.2831853 * radius / look.x;
    let major = abs(round(pos) % 6.0) < 0.5;
    let into = (x.w - radius) / y.w;
    lines = groove(off, 0.008, foot) * f32(into < select(0.35, 0.75, major));
  }
  var layer: Layer;
  if (params.look == 1u) {
    // Bronze: solid, giving no light of its own, lit only by the core (a narrow highlight sliding
    // along the band as it turns) and faintly by the stars; its ticks cut in, darker. Every other
    // ring a darker bronze.
    let bronze = state.palette[8].rgb * select(1.0, 0.7, i % 2u == 1u);
    let f = 0.04 + 0.96 * pow(1.0 - max(dot(n, -d), 0.0), 5.0);
    var c = bronze * light_at(p, n, true) * 0.5 + mix(bronze, state.palette[9].rgb * 0.6, 0.4) * env(reflect(d, n)) * (1.2 + 3.0 * f) + core_colour() * core_glint(p, n, d, 120.0) * 0.5;
    c *= 1.0 - 0.6 * lines;
    layer.front = c;
    layer.lets = vec3f(0.0);
    return layer;
  }
  let cosv = abs(dot(n, d));
  let slant = pow(1.0 - cosv, 3.0);
  // A faint rose at a slant, as thin films colour.
  let tint = mix(brass_colour(), state.palette[0].rgb, 0.35 * slant) * select(1.0, 0.7, look.y > 0.5);
  let surface = tint * light_at(p, n, true) * 0.25 + tint * env(reflect(d, n)) * (0.3 + 0.7 * slant) + tint * core_glint(p, n, d, 40.0);
  let alpha = mix(0.65, 0.92, slant);
  layer.front = surface * alpha + state.ring_tint[i].rgb * (rim * 0.9 + lines * 0.8) * params.ring_glow;
  layer.lets = mix(vec3f(1.0), brass_colour(), 0.45) * (1.0 - alpha);
  return layer;
}

// ---- Space ----

// The stars in one size of cell over the sky: the sky cut as a cube's six faces, each into
// `cells` squares (so no cell is drawn out thin), a star in some squares, a point a pixel
// and a half wide, some brighter, warm or cool, twinkling a little, slowly.
fn stars(d: vec3f, pixel: f32, cells: f32, share: f32, bright: f32) -> vec3f {
  let a = abs(d);
  var face = 0.0;
  var uv = d.yz / a.x;
  if (a.y >= a.x && a.y >= a.z) {
    face = 1.0;
    uv = d.zx / a.y;
  } else if (a.z >= a.x) {
    face = 2.0;
    uv = d.xy / a.z;
  }
  let major = select(select(d.x, d.y, face > 0.5), d.z, face > 1.5);
  face += select(0.0, 3.0, major < 0.0);
  let q = uv * cells;
  let cell = floor(q);
  let key = cell + vec2f(face * 1013.0, cells);
  if (hash(key) > share) {
    return vec3f(0.0);
  }
  let at = cell + 0.2 + 0.6 * vec2f(hash(key + 0.1), hash(key + 0.2));
  // How far from the star, as an angle (a cell is about 2 / cells across).
  let off = length(q - at) * 2.0 / cells;
  // Spread over a pixel and more, its light the same in all: a point narrower than a pixel is
  // caught whole by one pixel or only in part, by turns, as the camera moves, and flickers.
  let size = max(pixel * 1.3, 0.0002);
  let glow = exp(-off * off / (size * size)) * 0.3 * pixel * pixel / (size * size);
  let tint = mix(state.palette[6].rgb, state.palette[7].rgb, hash(key + 0.4));
  let twinkle = 0.92 + 0.08 * sin(frame.time * mix(0.3, 0.8, hash(key + 0.5)) + hash(key + 0.6) * 6.28);
  return tint * glow * bright * mix(0.25, 1.0, pow(hash(key + 0.7), 4.0)) * twinkle;
}

// Space along d: near black deep indigo; a band of nebula across it, violet and teal, brightest
// where the starlight comes from; and stars of two sizes.
fn sky(d: vec3f, pixel: f32) -> vec3f {
  var c = state.palette[5].rgb;
  let band = exp(-pow(dot(d, normalize(vec3f(0.3, 0.85, -0.4))), 2.0) / 0.08);
  let n = rv_noise(d.xy * 3.0 + 1.3) + rv_noise(d.yz * 3.0 + 4.1) + rv_noise(d.zx * 3.0 + 7.7);
  let fine = rv_noise(d.xy * 9.0) + rv_noise(d.yz * 9.0) + rv_noise(d.zx * 9.0);
  let cloud = smoothstep(-0.3, 1.2, n + 0.35 * fine) * band;
  let toward = max(dot(d, normalize(STARLIGHT)), 0.0);
  c += mix(state.palette[3].rgb, state.palette[4].rgb, smoothstep(-0.5, 0.8, fine)) * cloud * (0.6 + 1.4 * toward) * params.nebula;
  // (Dense enough that the long lens's narrow view is still full of them.)
  // (Behind the rings drawn in marks, the sky keeps back: fewer, fainter stars, less nebula.)
  let keep = select(1.0, 0.45, params.look == 3u);
  c = mix(state.palette[5].rgb, c, keep);
  c += stars(d, pixel, 90.0, 0.2 * keep, 2.2) * keep;
  c += stars(d, pixel, 260.0, 0.12 * keep, 0.7) * (0.35 + cloud * 2.5) * keep;
  return c;
}
