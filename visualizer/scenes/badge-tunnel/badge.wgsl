// The things in the tunnel, met exactly by a ray in each one's own frame (scaled by its size):
// badges, and flakes of leaf; and how they look. A badge is a lacquered token in the shape of a
// tortoiseshell (kikko, a hexagon flat on top and bottom), printed in gold or silver leaf with
// the shell's double frame and a family crest (kamon) within it, front and back. It keeps to
// that: one shape, three lacquers, two leaves, twelve crests; a soft deep gloss and no more.

// The lacquers: 0 black, 1 vermilion (shu), 2 indigo; and the leaves: 0 gold, 1 silver.
fn lacquer_colour(k: f32) -> vec3f {
  return state.colours[select(select(12u, 13u, k > 0.5), 14u, k > 1.5)].rgb;
}

fn leaf_colour(k: f32) -> vec3f {
  return state.colours[select(15u, 16u, k > 0.5)].rgb;
}

// Half the badge's thickness, against its apothem (1).
const THICK: f32 = 0.12;
// From the middle (0) out to the edge (1): the frame's two lines, and how far the crest reaches.
const FRAME_OUT: vec2f = vec2f(0.82, 0.885);
const FRAME_IN: vec2f = vec2f(0.745, 0.765);
const CREST_R: f32 = 0.62;
// The key light, from above and to one side of the camera.
const KEY: vec3f = vec3f(-0.42, 0.62, -0.66);

// The way slab k of an n-gon faces (its apothem along it): flat on top and bottom.
fn side_dir(k: f32, n: f32) -> vec2f {
  let a = (k * 2.0 + 1.0) * 3.14159265 / n;
  return vec2f(cos(a), sin(a));
}

// A ray (in the badge's frame, scaled) against the badge of shape n (0 round): distance and the
// normal it meets, or t 1e9.
fn badge_hit(ro: vec3f, rd: vec3f, n: f32) -> vec4f {
  let miss = vec4f(1e9, 0.0, 0.0, 0.0);
  var t0 = -1e9;
  var t1 = 1e9;
  var nrm = vec3f(0.0);
  // The two faces.
  if (abs(rd.z) < 1e-7) {
    if (abs(ro.z) > THICK) {
      return miss;
    }
  } else {
    let a = (-THICK - ro.z) / rd.z;
    let b = (THICK - ro.z) / rd.z;
    t0 = min(a, b);
    t1 = max(a, b);
    nrm = vec3f(0.0, 0.0, -sign(rd.z));
  }
  if (n < 0.5) {
    // A round badge: a cylinder of radius 1.
    let aa = dot(rd.xy, rd.xy);
    let bb = dot(ro.xy, rd.xy);
    let cc = dot(ro.xy, ro.xy) - 1.0;
    let disc = bb * bb - aa * cc;
    if (disc < 0.0 || aa < 1e-12) {
      return miss;
    }
    let sq = sqrt(disc);
    let tn = (-bb - sq) / aa;
    if (tn > t0) {
      t0 = tn;
      nrm = vec3f(normalize(ro.xy + rd.xy * tn), 0.0);
    }
    t1 = min(t1, (-bb + sq) / aa);
  } else {
    // A regular n-gon: n / 2 slabs of half width 1.
    for (var k = 0.0; k < n * 0.5; k += 1.0) {
      let u = side_dir(k, n);
      let o = dot(ro.xy, u);
      let dd = dot(rd.xy, u);
      if (abs(dd) < 1e-7) {
        if (abs(o) > 1.0) {
          return miss;
        }
        continue;
      }
      let a = (-1.0 - o) / dd;
      let b = (1.0 - o) / dd;
      let tn = min(a, b);
      if (tn > t0) {
        t0 = tn;
        nrm = vec3f(-u * sign(dd), 0.0);
      }
      t1 = min(t1, max(a, b));
    }
  }
  if (t0 > t1 || t0 < 0.0) {
    return miss;
  }
  return vec4f(t0, nrm);
}

// A cube of half size 1.
fn cube_hit(ro: vec3f, rd: vec3f) -> vec4f {
  let m = 1.0 / rd;
  let k = abs(m);
  let n = m * ro;
  let a = -n - k;
  let b = -n + k;
  let tn = max(max(a.x, a.y), a.z);
  let tf = min(min(b.x, b.y), b.z);
  if (tn > tf || tn < 0.0) {
    return vec4f(1e9, 0.0, 0.0, 0.0);
  }
  let nrm = -sign(rd) * step(a.yzx, a.xyz) * step(a.zxy, a.xyz);
  return vec4f(tn, nrm);
}

// How far a point of the face is from the middle (0) to the edge (1), and the way out there.
fn face_reach(q: vec2f, n: f32) -> vec3f {
  if (n < 0.5) {
    let r = length(q);
    return vec3f(r, q / max(r, 1e-5));
  }
  var best = vec3f(0.0);
  for (var k = 0.0; k < n * 0.5; k += 1.0) {
    let u = side_dir(k, n);
    let o = dot(q, u);
    if (abs(o) > best.x) {
      best = vec3f(abs(o), u * sign(o));
    }
  }
  return best;
}

// What the tunnel mirrors in a shiny surface facing r: dark below and brighter above, the core's
// light ahead, a soft light behind the camera, and two long strip lights across the sky, so that
// a domed face or a rounded bevel turning past catches a streak of light, as a glossy badge does
// under a studio's lights.
fn env(r: vec3f) -> vec3f {
  let ahead = max(r.z, 0.0);
  // Mostly dark, so what is bright in it reads as gloss.
  var e = mix(state.colours[17].rgb, state.colours[18].rgb, smoothstep(-0.6, 0.8, r.y));
  e += state.colours[19].rgb * (pow(ahead, 10.0) * 4.0 + pow(ahead, 2.0) * 0.25);
  e += state.colours[20].rgb * smoothstep(0.3, 1.0, -r.z) * smoothstep(-0.4, 0.6, r.y) * 0.35;
  let a = dot(r, vec3f(0.64, -0.36, 0.68));
  let b = dot(r, vec3f(-0.5, -0.66, 0.56));
  e += state.colours[21].rgb * exp(-a * a / 0.012) * smoothstep(-0.2, 0.3, r.y) * 3.0;
  e += state.colours[22].rgb * exp(-b * b / 0.008) * smoothstep(0.4, -0.3, r.x) * 2.0;
  return e;
}

fn fresnel(cosv: f32, f0: f32) -> f32 {
  return f0 + (1.0 - f0) * pow(1.0 - clamp(cosv, 0.0, 1.0), 5.0);
}

// The key light's highlight on a surface facing n, `sharp` how tight.
fn glint(n: vec3f, d: vec3f, sharp: f32) -> f32 {
  return pow(max(dot(reflect(d, n), normalize(KEY)), 0.0), sharp);
}

// Lacquer: its colour, softly lit, under a deep clear gloss that mirrors the tunnel and catches
// the key light in a small sharp point.
fn lacquer(col: vec3f, n: vec3f, d: vec3f) -> vec3f {
  let cosv = max(dot(n, -d), 0.0);
  let lit = 0.35 + 0.65 * max(dot(n, normalize(KEY)), 0.0);
  return col * lit + env(reflect(d, n)) * fresnel(cosv, 0.045) + state.colours[23].rgb * glint(n, d, 300.0) * 1.5;
}

// Leaf: metal, its surface a little uneven, so it lights more broadly than the lacquer round it
// and never goes dark: some of it always catches the light.
fn leaf(col: vec3f, n: vec3f, d: vec3f) -> vec3f {
  return col * env(reflect(d, n)) * 0.9 + col * glint(n, d, 12.0) * 1.2 + col * 0.3;
}

// How far crest k turns on each counted hit: by its symmetry, so it always comes to rest upright
// (the waves do not turn).
fn crest_turn(k: f32) -> f32 {
  var order = array<f32, 12>(3.0, 4.0, 6.0, 6.0, 4.0, 5.0, 0.0, 3.0, 8.0, 2.0, 2.0, 4.0);
  let o = order[u32(clamp(k, 0.0, 11.0))];
  return select(0.0, 6.2831853 / o, o > 0.5);
}

// Coverage of the ring between radii a and b, with edges a pixel (f) soft.
fn band(r: f32, a: f32, b: f32, f: f32) -> f32 {
  return smoothstep(a - f * 0.5, a + f * 0.5, r) * (1.0 - smoothstep(b - f * 0.5, b + f * 0.5, r));
}

// Badge i, met at distance t along d: n the normal and q the point, both in its own frame (q
// scaled by its size).
fn shade_badge(i: u32, d: vec3f, t: f32, nl: vec3f, q: vec3f, pixel: f32) -> vec3f {
  let rx = state.rx[i];
  let ry = state.ry[i];
  let rz = state.rz[i];
  let size = state.pos[i].w;
  let body = lacquer_colour(ry.w % 8.0);
  let gold = leaf_colour(floor(ry.w / 8.0));
  let to_world = mat3x3f(rx.xyz, ry.xyz, rz.xyz);
  if (abs(nl.z) < 0.5) {
    // The side: lacquer, rounding into the faces at its edges, where it catches the light.
    let z = q.z / THICK;
    let bend = sign(z) * smoothstep(0.4, 1.0, abs(z)) * 1.2;
    return lacquer(body, normalize(to_world * normalize(nl + vec3f(0.0, 0.0, bend))), d);
  }
  let reach = face_reach(q.xy, 6.0);
  // The face, domed a very little, its edge rounded off.
  let edge = smoothstep(0.93, 1.0, reach.x);
  let n = normalize(to_world * normalize(vec3f(q.xy * 0.12 + reach.yz * edge * 1.4, nl.z)));
  // One pixel, in the badge's own measure.
  let f = max(t * pixel / size / max(abs(dot(n, d)), 0.25), 0.002);
  // The leaf: the shell's double frame, and the crest.
  var cover = max(band(reach.x, FRAME_OUT.x, FRAME_OUT.y, f), band(reach.x, FRAME_IN.x, FRAME_IN.y, f));
  let k = rz.w % 16.0;
  let way = select(-1.0, 1.0, rz.w >= 16.0);
  let a = state.turn * crest_turn(k) * way;
  var cq = q.xy / CREST_R;
  if (nl.z > 0.0) {
    // (Mirrored on the back, so it reads the right way round from either side.)
    cq.x = -cq.x;
  }
  cq = vec2f(cq.x * cos(a) - cq.y * sin(a), cq.x * sin(a) + cq.y * cos(a));
  if (dot(cq, cq) < 1.3) {
    let cell = vec2f(k % 4.0, floor(k / 4.0));
    let uv = vec2f(0.5 + cq.x * 0.43, 0.5 - cq.y * 0.43);
    // The field is in the cell's pixels; one screen pixel covers this many of them.
    let px = max(f / CREST_R * 0.43 * 256.0, 1.0);
    let v = textureSampleLevel(asset_crests, linear_clamp, (cell + uv) / 4.0, log2(px)).r;
    cover = max(cover, smoothstep(-0.5 * px, 0.5 * px, (v - 0.5) * 32.0));
  }
  return mix(lacquer(body, n, d), leaf(gold, n, d), cover);
}

// A flake of gold or silver leaf, thin and flat.
fn flake_hit(ro: vec3f, rd: vec3f) -> vec4f {
  let s = vec3f(1.0, 1.0, 0.08);
  return cube_hit(ro / s, rd / s);
}

fn shade_flake(i: u32, d: vec3f, nl: vec3f) -> vec3f {
  let rx = state.rx[i];
  let ry = state.ry[i];
  let rz = state.rz[i];
  let n = normalize(mat3x3f(rx.xyz, ry.xyz, rz.xyz) * nl);
  return leaf(leaf_colour(floor(ry.w / 8.0)), n, d);
}

// An emoji: its outline (the atlas's alpha) cut out of a slab, as acrylic or a cookie is, a little
// thick; facing the camera, so only those off to the side of the picture show their edge.
const EMOJI_THICK: f32 = 0.07;

fn emoji_texel(q: vec2f, k: f32, back: bool, lod: f32) -> vec4f {
  var uv = q * 0.5 + 0.5;
  uv.y = 1.0 - uv.y;
  if (back) {
    uv.x = 1.0 - uv.x;
  }
  let cell = vec2f(k % 8.0, floor(k / 8.0));
  return textureSampleLevel(asset_emoji, linear_clamp, (cell + uv) / 8.0, lod);
}

fn inside_emoji(q: vec2f, k: f32, lod: f32) -> bool {
  return all(abs(q) < vec2f(1.0)) && emoji_texel(q, k, false, lod).a > 0.5;
}

// Where the ray meets the emoji: its face where it enters the slab, if the outline is there;
// else its edge, the first of a few points through the slab inside the outline (then halved
// once towards the outside). The normal of the edge is the way the outline faces there.
fn emoji_hit(ro: vec3f, rd: vec3f, k: f32, lod: f32) -> vec4f {
  let miss = vec4f(1e9, 0.0, 0.0, 0.0);
  if (abs(rd.z) < 1e-7) {
    return miss;
  }
  let a = (-EMOJI_THICK - ro.z) / rd.z;
  let b = (EMOJI_THICK - ro.z) / rd.z;
  let t0 = min(a, b);
  let t1 = max(a, b);
  if (t1 < 0.0) {
    return miss;
  }
  let q0 = ro.xy + rd.xy * t0;
  if (all(abs(q0) < vec2f(1.0)) && emoji_texel(q0, k, false, lod).a > 0.3) {
    return vec4f(t0, 0.0, 0.0, -sign(rd.z));
  }
  var last = t0;
  // Quality: how many steps look through an emoji's thickness for its cut edge (1: six; 0.75:
  // five; 0.5: four; 0.25: three; 0: two). Fewer steps miss thin parts of the outline, nothing
  // else.
  let steps = 2 + i32(round(4.0 * frame.quality));
  for (var s = 1; s <= steps; s++) {
    let t = mix(t0, t1, f32(s) / f32(steps));
    if (inside_emoji(ro.xy + rd.xy * t, k, lod)) {
      let mid = 0.5 * (last + t);
      let th = select(t, mid, inside_emoji(ro.xy + rd.xy * mid, k, lod));
      let q = ro.xy + rd.xy * th;
      let e = 0.02;
      let gx = emoji_texel(q + vec2f(e, 0.0), k, false, lod).a - emoji_texel(q - vec2f(e, 0.0), k, false, lod).a;
      let gy = emoji_texel(q + vec2f(0.0, e), k, false, lod).a - emoji_texel(q - vec2f(0.0, e), k, false, lod).a;
      // (The atlas's v runs down, so its slope in y is the other way.)
      let g = vec2f(gx, -gy);
      let n = select(-normalize(rd.xy), -normalize(g), dot(g, g) > 1e-8);
      return vec4f(th, n, 0.0);
    }
    last = t;
  }
  return miss;
}

// The emoji's own colours (it is drawn lit already), kept low for the dark; its edge the colour
// at the outline, darker, lit from the key light; and how much of the pixel the face covers, so
// its outline is smooth.
fn shade_emoji(i: u32, d: vec3f, nl: vec3f, q: vec3f, lod: f32) -> vec4f {
  let rx = state.rx[i];
  let ry = state.ry[i];
  let rz = state.rz[i];
  let n = normalize(mat3x3f(rx.xyz, ry.xyz, rz.xyz) * nl);
  if (abs(nl.z) < 0.5) {
    let e = emoji_texel(q.xy - nl.xy * 0.02, rz.w, false, lod);
    let c = e.rgb / max(e.a, 1e-3);
    return vec4f(c * (0.25 + 0.45 * max(dot(n, normalize(KEY)), 0.0)), 1.0);
  }
  let e = emoji_texel(q.xy, rz.w, false, lod);
  let c = e.rgb / max(e.a, 1e-3) * (0.65 + 0.15 * abs(dot(n, d)));
  return vec4f(c, smoothstep(0.3, 0.7, e.a));
}
