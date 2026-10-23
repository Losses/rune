// The runtime's library for scenes. Every name starts with rv_.

const RV_PI: f32 = 3.14159265358979;
const RV_TAU: f32 = 6.28318530717959;

// ---- Hashes and random numbers ----

// PCG hash of one word.
fn rv_hash(v: u32) -> u32 {
  let state = v * 747796405u + 2891336453u;
  let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
  return (word >> 22u) ^ word;
}

fn rv_hash2(v: vec2u) -> u32 {
  return rv_hash(v.x ^ rv_hash(v.y));
}

// 0..1 from a word.
fn rv_unit(v: u32) -> f32 {
  return f32(rv_hash(v) >> 8u) / 16777216.0;
}

// 0..1 from a point; lattice points of the same integer coordinates give the same number.
fn rv_rand2(p: vec2f) -> f32 {
  return f32(rv_hash2(bitcast<vec2u>(vec2i(floor(p)))) >> 8u) / 16777216.0;
}

// 0..1 from any float (by its bits).
fn rv_rand1(x: f32) -> f32 {
  return rv_unit(bitcast<u32>(x));
}

// ---- Noise ----

fn rv_value_noise(p: vec2f) -> f32 {
  let i = floor(p);
  let f = fract(p);
  let u = f * f * (3.0 - 2.0 * f);
  let a = rv_rand2(i);
  let b = rv_rand2(i + vec2f(1.0, 0.0));
  let c = rv_rand2(i + vec2f(0.0, 1.0));
  let d = rv_rand2(i + vec2f(1.0, 1.0));
  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn rv_gradient(i: vec2f) -> vec2f {
  let a = rv_rand2(i) * RV_TAU;
  return vec2f(cos(a), sin(a));
}

// Gradient noise, about -0.7..0.7.
fn rv_noise(p: vec2f) -> f32 {
  let i = floor(p);
  let f = fract(p);
  let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
  let a = dot(rv_gradient(i), f);
  let b = dot(rv_gradient(i + vec2f(1.0, 0.0)), f - vec2f(1.0, 0.0));
  let c = dot(rv_gradient(i + vec2f(0.0, 1.0)), f - vec2f(0.0, 1.0));
  let d = dot(rv_gradient(i + vec2f(1.0, 1.0)), f - vec2f(1.0, 1.0));
  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn rv_fbm(p: vec2f, octaves: i32) -> f32 {
  var sum = 0.0;
  var amp = 0.5;
  var q = p;
  for (var i = 0; i < octaves; i++) {
    sum += amp * rv_noise(q);
    q = q * 2.03 + vec2f(17.1, 9.3);
    amp *= 0.5;
  }
  return sum;
}

// ---- Shapes (signed distances) ----

fn rv_sd_circle(p: vec2f, r: f32) -> f32 {
  return length(p) - r;
}

fn rv_sd_box(p: vec2f, half_size: vec2f) -> f32 {
  let q = abs(p) - half_size;
  return length(max(q, vec2f(0.0))) + min(max(q.x, q.y), 0.0);
}

fn rv_sd_round_box(p: vec2f, half_size: vec2f, r: f32) -> f32 {
  return rv_sd_box(p, half_size - vec2f(r)) - r;
}

fn rv_sd_segment(p: vec2f, a: vec2f, b: vec2f) -> f32 {
  let pa = p - a;
  let ba = b - a;
  let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
  return length(pa - ba * h);
}

// 1 inside a..b, with edges aa wide.
fn rv_band(x: f32, a: f32, b: f32, aa: f32) -> f32 {
  return smoothstep(a - aa, a + aa, x) * smoothstep(b + aa, b - aa, x);
}

fn rv_rotate(p: vec2f, a: f32) -> vec2f {
  let c = cos(a);
  let s = sin(a);
  return vec2f(c * p.x - s * p.y, s * p.x + c * p.y);
}

// ---- Colour ----

fn rv_srgb_to_linear(c: vec3f) -> vec3f {
  return select(pow((c + 0.055) / 1.055, vec3f(2.4)), c / 12.92, c <= vec3f(0.04045));
}

fn rv_linear_to_srgb(c: vec3f) -> vec3f {
  return select(1.055 * pow(c, vec3f(1.0 / 2.4)) - 0.055, c * 12.92, c <= vec3f(0.0031308));
}

fn rv_luma(c: vec3f) -> f32 {
  return dot(c, vec3f(0.2126, 0.7152, 0.0722));
}

fn rv_linear_to_oklab(c: vec3f) -> vec3f {
  let l = 0.4122214708 * c.r + 0.5363325363 * c.g + 0.0514459929 * c.b;
  let m = 0.2119034982 * c.r + 0.6806995451 * c.g + 0.1073969566 * c.b;
  let s = 0.0883024619 * c.r + 0.2817188376 * c.g + 0.6299787005 * c.b;
  let lms = pow(max(vec3f(l, m, s), vec3f(0.0)), vec3f(1.0 / 3.0));
  return vec3f(
    0.2104542553 * lms.x + 0.7936177850 * lms.y - 0.0040720468 * lms.z,
    1.9779984951 * lms.x - 2.4285922050 * lms.y + 0.4505937099 * lms.z,
    0.0259040371 * lms.x + 0.7827717662 * lms.y - 0.8086757660 * lms.z);
}

fn rv_oklab_to_linear(c: vec3f) -> vec3f {
  let l = c.x + 0.3963377774 * c.y + 0.2158037573 * c.z;
  let m = c.x - 0.1055613458 * c.y - 0.0638541728 * c.z;
  let s = c.x - 0.0894841775 * c.y - 1.2914855480 * c.z;
  let lms = vec3f(l * l * l, m * m * m, s * s * s);
  return vec3f(
    4.0767416621 * lms.x - 3.3077115913 * lms.y + 0.2309699292 * lms.z,
    -1.2684380046 * lms.x + 2.6097574011 * lms.y - 0.3413193965 * lms.z,
    -0.0041960863 * lms.x - 0.7034186147 * lms.y + 1.7076147010 * lms.z);
}

// Turn the hue by `a` radians, keeping lightness (in OKLab).
fn rv_hue_rotate(c: vec3f, a: f32) -> vec3f {
  let lab = rv_linear_to_oklab(c);
  return max(rv_oklab_to_linear(vec3f(lab.x, rv_rotate(lab.yz, a))), vec3f(0.0));
}

// ---- Rhythm ----

// A spike at phase 0 (and every whole number) falling off with `sharpness`.
fn rv_pulse(phase: f32, sharpness: f32) -> f32 {
  return exp(-fract(phase) * sharpness);
}

// What is left of 1 after t seconds, halving every half_life seconds.
fn rv_decay(t: f32, half_life: f32) -> f32 {
  return exp2(-max(t, 0.0) / max(half_life, 1e-4));
}
