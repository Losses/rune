// Space round the sphere, and what meets a ray exactly (no marching): the planet at its heart. The
// space behind: stars.

const MISS: vec4f = vec4f(1e9, 0.0, 0.0, 0.0);

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
  // (Further than six sizes off, under 1e-15 of its light reaches the pixel: left out.)
  if (off > 6.0 * size) {
    return vec3f(0.0);
  }
  let glow = exp(-off * off / (size * size)) * 0.3 * pixel * pixel / (size * size);
  let tint = mix(state.palette[6].rgb, state.palette[7].rgb, hash(key + 0.4));
  // (The twinkle stays on the clock: it is light, not motion, so it goes on in silence.)
  let twinkle = 0.92 + 0.08 * sin(frame.time * mix(0.3, 0.8, hash(key + 0.5)) + hash(key + 0.6) * 6.28);
  return tint * glow * bright * mix(0.25, 1.0, pow(hash(key + 0.7), 4.0)) * twinkle;
}

// Space along d: near black deep indigo, and stars of two sizes (no nebula: it looked artificial).
fn sky(d: vec3f, pixel: f32) -> vec3f {
  // (Dense enough that the long lens's narrow view is still full of them.)
  // (Behind the rings' marks, the sky keeps back: fewer, fainter stars.)
  let keep = 0.45;
  var c = state.palette[5].rgb;
  c += stars(d, pixel, 90.0, 0.2 * keep, 2.2) * keep;
  c += stars(d, pixel, 260.0, 0.12 * keep, 0.7) * 0.35 * keep;
  return c;
}
