// The runtime's post-processing: bloom on a mip chain (13-tap downsampling with a Karis
// average on the first level, tent upsampling), then exposure, chromatic aberration, tone
// mapping, vignette, grain and dithering into the 8-bit frame. The Rust side puts the structs
// Frame and Post and the constants LEVELS and TONEMAP (0 AgX, 1 ACES, 2 none) in front.
//
// The lens effects of a scene that lists them (struct Lens, see scene/lens.rs; all zeros
// otherwise, and then nothing below costs a tap; each effect is started at a moment and runs
// its course, so these are strengths that rise and fall). The flare and the block frame start
// from the picture at a quarter of the size (`lens_source`) and the frame's key: its brightest
// luma, its mean and where the brightest is (`lens_cells`, `lens_key`), so that they find a
// night scene's highlights as well as a bright one's. The flare's streaks and ghosts are drawn
// at that size (`flare_wide`, `flare`); the composite lays the flare, the light leak and the
// block frame over the tone-mapped picture, in display values, so they lift the dark instead of
// greying it, and none reaches full white.

@group(0) @binding(0) var<storage, read> post: Post;
@group(0) @binding(1) var<uniform> frame: Frame;
@group(0) @binding(2) var<uniform> lens: Lens;
@group(1) @binding(0) var source: texture_2d<f32>;
@group(1) @binding(1) var smp: sampler;
@group(1) @binding(2) var bloom: texture_2d<f32>;
@group(1) @binding(3) var blue_noise: texture_2d<f32>;
// The flare, in the composite; in `flare` its wide streaks from `flare_wide`.
@group(1) @binding(4) var flare_tex: texture_2d<f32>;
// The frame's key (1 by 1): the brightest luma, the mean luma, and where the brightest is (uv).
// In `lens_key` the one of the frame before.
@group(1) @binding(5) var key_tex: texture_2d<f32>;
// The picture at a quarter of the size, exposed, with its luma in alpha: for the block frame.
// In `lens_source` the frame before's key is at binding 5.
@group(1) @binding(6) var lens_tex: texture_2d<f32>;

// Which lens effects are on. `flare` is the strength of the glare (0 off), `sweep` how far its
// streak has swept out (0 to 1); `fresh` is 1 when the key is taken as it is (the first frame
// with a key after one without); `leak` how much of the light
// leak is in (0 to 1) with its source at (leak_x, leak_y) in uv, of radius leak_size (a share of
// the long side), a phase that grows over the leak and how hot it burns at its edge (`burn`);
// `blocks` the strength of a colour-block frame (0 off), `inverted` 1 on its negative.
struct Lens {
  flare: f32,
  sweep: f32,
  fresh: f32,
  blocks: f32,
  leak: f32,
  leak_x: f32,
  leak_y: f32,
  leak_size: f32,
  phase: f32,
  burn: f32,
  inverted: f32,
  unused: f32,
}

struct Varyings {
  @builtin(position) position: vec4f,
  @location(0) uv: vec2f,
}

@vertex
fn vertex(@builtin(vertex_index) i: u32) -> Varyings {
  let xy = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
  var out: Varyings;
  out.position = vec4f(xy * 2.0 - 1.0, 0.0, 1.0);
  out.uv = vec2f(xy.x, 1.0 - xy.y);
  return out;
}

fn luma(c: vec3f) -> f32 {
  return dot(c, vec3f(0.2126, 0.7152, 0.0722));
}

fn tap(uv: vec2f, texel: vec2f, x: f32, y: f32) -> vec3f {
  return textureSampleLevel(source, smp, uv + texel * vec2f(x, y), 0.0).rgb;
}

// Four 2x2 boxes around the centre and one in it, as in Jimenez's "Next Generation Post
// Processing in Call of Duty". With `karis` each box is weighted by 1 / (1 + luma) so a few
// very bright pixels cannot flicker the whole bloom.
fn downsample(uv: vec2f, karis: bool) -> vec3f {
  let texel = 1.0 / vec2f(textureDimensions(source));
  let a = tap(uv, texel, -2.0, 2.0);
  let b = tap(uv, texel, 0.0, 2.0);
  let c = tap(uv, texel, 2.0, 2.0);
  let d = tap(uv, texel, -2.0, 0.0);
  let e = tap(uv, texel, 0.0, 0.0);
  let f = tap(uv, texel, 2.0, 0.0);
  let g = tap(uv, texel, -2.0, -2.0);
  let h = tap(uv, texel, 0.0, -2.0);
  let i = tap(uv, texel, 2.0, -2.0);
  let j = tap(uv, texel, -1.0, 1.0);
  let k = tap(uv, texel, 1.0, 1.0);
  let l = tap(uv, texel, -1.0, -1.0);
  let m = tap(uv, texel, 1.0, -1.0);
  let boxes = array<vec3f, 5>(
    (j + k + l + m) * 0.25,
    (a + b + d + e) * 0.25,
    (b + c + e + f) * 0.25,
    (d + e + g + h) * 0.25,
    (e + f + h + i) * 0.25,
  );
  let weights = array<f32, 5>(0.5, 0.125, 0.125, 0.125, 0.125);
  var sum = vec3f(0.0);
  var total = 0.0;
  for (var n = 0; n < 5; n++) {
    var w = weights[n];
    if (karis) {
      w /= 1.0 + luma(boxes[n]);
    }
    sum += boxes[n] * w;
    total += w;
  }
  return sum / total;
}

// What of c is above the threshold, with a soft knee half the threshold wide.
fn above_threshold(c: vec3f) -> vec3f {
  let t = max(post.bloom_threshold, 0.0);
  let knee = 0.5 * t + 1e-4;
  let b = max(max(c.r, c.g), c.b);
  var soft = clamp(b - t + knee, 0.0, 2.0 * knee);
  soft = soft * soft / (4.0 * knee);
  return c * max(soft, b - t) / max(b, 1e-4);
}

// The app's accent in linear light with its brightest channel at 1: the one colour family the
// lens effects are drawn in. A warm orange before the app has told one.
fn theme() -> vec3f {
  var c = frame.accent.rgb;
  let m = max(c.r, max(c.g, c.b));
  if (frame.accent.a < 0.5 || m < 1e-3) {
    c = vec3f(1.0, 0.35, 0.08);
  }
  return c / max(c.r, max(c.g, c.b));
}

@fragment
fn first_down(v: Varyings) -> @location(0) vec4f {
  return vec4f(above_threshold(downsample(v.uv, true)), 1.0);
}

@fragment
fn down(v: Varyings) -> @location(0) vec4f {
  return vec4f(downsample(v.uv, false), 1.0);
}

// Added onto the level above (blending one + one), weighted by the radius: a larger radius
// keeps more of the wide levels.
@fragment
fn up(v: Varyings) -> @location(0) vec4f {
  let texel = 1.0 / vec2f(textureDimensions(source));
  var sum = tap(v.uv, texel, 0.0, 0.0) * 4.0;
  sum += (tap(v.uv, texel, -1.0, 0.0) + tap(v.uv, texel, 1.0, 0.0) + tap(v.uv, texel, 0.0, -1.0) + tap(v.uv, texel, 0.0, 1.0)) * 2.0;
  sum += tap(v.uv, texel, -1.0, -1.0) + tap(v.uv, texel, 1.0, -1.0) + tap(v.uv, texel, -1.0, 1.0) + tap(v.uv, texel, 1.0, 1.0);
  return vec4f(sum / 16.0 * clamp(post.bloom_radius, 0.0, 1.0), 1.0);
}

// ---- The lens effects' own passes, drawn only while the flare or a block frame is on ----

// A 4 by 4 box of the picture's luma at uv, exposed: two taps each way between texels.
fn box_luma(uv: vec2f, texel: vec2f) -> f32 {
  let a = tap(uv, texel, -1.0, -1.0) + tap(uv, texel, 1.0, -1.0);
  let b = tap(uv, texel, -1.0, 1.0) + tap(uv, texel, 1.0, 1.0);
  return luma(a + b) * 0.25 * exp2(post.exposure);
}

// The picture at a quarter of the internal size, exposed, before bloom and tone mapping: its
// luma in alpha, where the frame's key and the block frame start from (the 13 taps with the
// Karis average leave no halftone dot or lone bright pixel to speak of); and in red, while the
// flare is on, what throws the flare's streaks: the highlights that are small up and down. A
// lamp or a thin line throws a full streak; a large bright area only a faint one at its top and
// bottom edges, so a frame of big signs is not barred with thick bands. The highlights are
// found against the key of the frame before, which the next pass is about to replace.
@fragment
fn lens_source(v: Varyings) -> @location(0) vec4f {
  let l = luma(downsample(v.uv, true)) * exp2(post.exposure);
  var spot = 0.0;
  if (lens.flare > 0.0) {
    let key = textureLoad(key_tex, vec2i(0), 0);
    let texel = 1.0 / vec2f(textureDimensions(source));
    var around = 0.0;
    for (var i = 1; i <= 2; i++) {
      // Two and four texels of the quarter size up and down.
      let y = vec2f(0.0, f32(i) * 8.0 * texel.y);
      around += highlight(box_luma(v.uv + y, texel), key) + highlight(box_luma(v.uv - y, texel), key);
    }
    let alone = 1.0 - around * 0.25;
    spot = highlight(l, key) * alone * alone;
  }
  return vec4f(spot, 0.0, 0.0, l);
}

// How many cells the key divides the frame into each way, and the taps a cell takes each way.
const CELLS: i32 = 8;
const CELL_TAPS: i32 = 8;

// Of one of 8 by 8 cells of the quarter-size picture: its brightest luma, its mean luma and
// where its brightest is (uv).
@fragment
fn lens_cells(v: Varyings) -> @location(0) vec4f {
  let cell = floor(v.position.xy);
  var most = 0.0;
  var sum = 0.0;
  var at = (cell + 0.5) / f32(CELLS);
  for (var j = 0; j < CELL_TAPS; j++) {
    for (var i = 0; i < CELL_TAPS; i++) {
      let uv = (cell + (vec2f(f32(i), f32(j)) + 0.5) / f32(CELL_TAPS)) / f32(CELLS);
      let l = textureSampleLevel(source, smp, uv, 0.0).a;
      sum += l;
      if (l > most) {
        most = l;
        at = uv;
      }
    }
  }
  return vec4f(most, sum / f32(CELL_TAPS * CELL_TAPS), at);
}

// The frame's key from its cells: the brightest luma, the mean, and where the brightest is
// (the cells' brightest spots weighted steeply by how bright they are). It follows the frame
// before smoothly, so the flare does not jump between highlights, except when it is fresh
// (the first frame with a key after one without), which takes the picture as it is.
@fragment
fn lens_key(v: Varyings) -> @location(0) vec4f {
  var most = 0.0;
  var mean = 0.0;
  for (var j = 0; j < CELLS; j++) {
    for (var i = 0; i < CELLS; i++) {
      let c = textureLoad(source, vec2i(i, j), 0);
      most = max(most, c.r);
      mean += c.g;
    }
  }
  mean /= f32(CELLS * CELLS);
  var at = vec2f(0.0);
  var total = 0.0;
  for (var j = 0; j < CELLS; j++) {
    for (var i = 0; i < CELLS; i++) {
      let c = textureLoad(source, vec2i(i, j), 0);
      let w = pow(c.r / max(most, 1e-4), 16.0);
      at += c.ba * w;
      total += w;
    }
  }
  let measured = vec4f(most, mean, at / max(total, 1e-6));
  let last = textureLoad(key_tex, vec2i(0), 0);
  if (lens.fresh > 0.5 || last.r <= 0.0) {
    return measured;
  }
  return mix(last, measured, 1.0 - exp(-frame.dt * 6.0));
}

// How much a luma of the picture throws the flare, 0 to 1: the highlights against
// the frame (from three quarters of the way from its mean to its brightest), its brightest 1,
// whatever the scene's brightness, so a night scene throws streaks as well as a bright one.
fn highlight(l: f32, key: vec4f) -> f32 {
  let top = max(key.r, 0.05);
  let low = min(mix(key.g, top, 0.75), top * 0.9);
  return smoothstep(low, top, l);
}

fn spot_at(uv: vec2f) -> f32 {
  return textureSampleLevel(source, smp, uv, 0.0).r;
}

// The flare's wide streaks, first step: the spots averaged along the row over a tenth of the
// width each way, with an exponential falloff. Taps two texels apart between texels, so that
// each takes two.
@fragment
fn flare_wide(v: Varyings) -> @location(0) vec4f {
  let texel = 1.0 / f32(textureDimensions(source).x);
  let n = min(i32(ceil(0.1 / (2.0 * texel))), 48);
  var sum = spot_at(v.uv);
  var total = 1.0;
  for (var i = 1; i <= n; i++) {
    let x = (f32(i) * 2.0 - 0.5) * texel;
    let w = exp(-x / 0.04);
    sum += (spot_at(v.uv + vec2f(x, 0.0)) + spot_at(v.uv - vec2f(x, 0.0))) * w;
    total += 2.0 * w;
  }
  return vec4f(sum / total, 0.0, 0.0, 1.0);
}

// A ghost of the lens at uv: a soft disc with a brighter rim, its edge a little wider in red
// than in blue, as a lens fringes. It sits at `along` on the line from the centre to the
// brightest highlight `to` (1 at the highlight, -1 mirrored), of `radius` (a share of the short
// side).
fn ghost(uv: vec2f, to: vec2f, along: f32, radius: f32) -> vec3f {
  let at = 0.5 + to * along;
  let d = length((uv - at) * vec2f(frame.aspect, 1.0)) / (min(frame.aspect, 1.0) * radius);
  let r = d / vec3f(1.0, 0.95, 0.9);
  let disc = 1.0 - smoothstep(vec3f(0.8), vec3f(1.0), r);
  let rim = smoothstep(vec3f(0.55), vec3f(0.95), r);
  return disc * (0.35 + 0.65 * rim);
}

// The flare at a quarter of the size, before its strength: a sharp core of streaks along the
// rows of the spots, nearly white, with long soft tails in the theme colour; and a few
// ghosts along the line from the brightest highlight through the centre. In linear light; the
// composite eases it into display values.
@fragment
fn flare(v: Varyings) -> @location(0) vec4f {
  let uv = v.uv;
  let key = textureLoad(key_tex, vec2i(0), 0);
  let theme_colour = theme();
  let texel = 1.0 / f32(textureDimensions(source).x);

  // The core: a narrow gather, about 1 % of the width.
  let n = min(i32(ceil(0.03 / texel)), 24);
  var core = spot_at(uv);
  var total = 1.0;
  for (var i = 1; i <= n; i++) {
    let x = f32(i) * texel;
    let w = exp(-x / 0.008);
    core += (spot_at(uv + vec2f(x, 0.0)) + spot_at(uv - vec2f(x, 0.0))) * w;
    total += 2.0 * w;
  }
  core /= total;

  // The tails: the wide step gathered again over the whole width, 5 % apart.
  var tail = textureSampleLevel(flare_tex, smp, uv, 0.0).r;
  var tails = 1.0;
  for (var i = 1; i <= 20; i++) {
    let x = f32(i) * 0.05;
    let w = exp(-x / 0.3);
    tail += (textureSampleLevel(flare_tex, smp, uv + vec2f(x, 0.0), 0.0).r
      + textureSampleLevel(flare_tex, smp, uv - vec2f(x, 0.0), 0.0).r) * w;
    tails += 2.0 * w;
  }
  tail /= tails;
  let near = textureSampleLevel(flare_tex, smp, uv, 0.0).r;

  var c = core * 1.6 * mix(theme_colour, vec3f(1.0), 0.5);
  c += (pow(near, 0.7) * 0.3 + pow(tail, 0.7) * 0.3) * theme_colour;

  // The ghosts, in the theme colour a little lighter.
  let to = key.ba - 0.5;
  var ghosts = ghost(uv, to, -0.45, 0.07) * 0.22;
  ghosts += ghost(uv, to, -1.0, 0.16) * 0.12;
  ghosts += ghost(uv, to, 0.35, 0.035) * 0.3;
  ghosts += ghost(uv, to, -1.55, 0.1) * 0.1;
  c += ghosts * mix(theme_colour, vec3f(1.0), 0.25);
  return vec4f(c, 1.0);
}

// Troy Sobotka's AgX with Benjamin Wrensch's polynomial fit of its contrast curve. Takes linear
// Rec. 709 and gives display-encoded values.
fn agx(c: vec3f) -> vec3f {
  let inset = mat3x3f(
    vec3f(0.842479062253094, 0.0423282422610123, 0.0423756549057051),
    vec3f(0.0784335999999992, 0.878468636469772, 0.0784336),
    vec3f(0.0792237451477643, 0.0791661274605434, 0.879142973793104));
  let outset = mat3x3f(
    vec3f(1.19687900512017, -0.0528968517574562, -0.0529716355144438),
    vec3f(-0.0980208811401368, 1.15190312990417, -0.0980434501171241),
    vec3f(-0.0990297440797205, -0.0989611768448433, 1.15107367264116));
  let min_ev = -12.47393;
  let max_ev = 4.026069;
  var x = inset * max(c, vec3f(0.0));
  x = clamp(log2(max(x, vec3f(1e-10))), vec3f(min_ev), vec3f(max_ev));
  x = (x - min_ev) / (max_ev - min_ev);
  let x2 = x * x;
  let x4 = x2 * x2;
  x = 15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x - 0.00232;
  // The look (Wrensch's agxLook): power for contrast, then saturation around the luma.
  let l = luma(x);
  x = pow(max(x, vec3f(0.0)), vec3f(post.contrast));
  x = l + post.saturation * (x - l);
  return clamp(outset * x, vec3f(0.0), vec3f(1.0));
}

fn encode_srgb(c: vec3f) -> vec3f {
  let x = clamp(c, vec3f(0.0), vec3f(1.0));
  return select(1.055 * pow(x, vec3f(1.0 / 2.4)) - 0.055, x * 12.92, x <= vec3f(0.0031308));
}

// Krzysztof Narkowicz's fit of the ACES film curve, then sRGB.
fn aces(c: vec3f) -> vec3f {
  let x = max(c, vec3f(0.0)) * 0.6;
  return encode_srgb((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14));
}

// Value noise: smooth, low in frequency, cheap; for the light leak's shape.
fn hash(p: vec2f) -> f32 {
  return fract(sin(dot(p, vec2f(127.1, 311.7))) * 43758.5453);
}

fn value_noise(p: vec2f) -> f32 {
  let i = floor(p);
  let f = fract(p);
  let u = f * f * (3.0 - 2.0 * f);
  return mix(
    mix(hash(i), hash(i + vec2f(1.0, 0.0)), u.x),
    mix(hash(i + vec2f(0.0, 1.0)), hash(i + vec2f(1.0, 1.0)), u.x),
    u.y);
}

// Screen blending in display values: lifts the dark, never past white.
fn screen(base: vec3f, over: vec3f) -> vec3f {
  return 1.0 - (1.0 - base) * (1.0 - clamp(over, vec3f(0.0), vec3f(1.0)));
}

// The light leak at uv, in display values: a large soft glow from its source beyond an edge,
// its outline torn by low-frequency noise that drifts with the phase, saturated in the theme
// colour going warmer towards the source, and burning hotter at the edge as it comes in. It
// blooms out as it comes in.
fn leak_light(uv: vec2f) -> vec3f {
  let long = max(frame.aspect, 1.0);
  let scale = vec2f(frame.aspect, 1.0) / long;
  let p = uv * scale;
  let d = (uv - vec2f(lens.leak_x, lens.leak_y)) * scale;
  let size = lens.leak_size * (0.6 + 0.4 * lens.leak);
  let drift = vec2f(cos(lens.phase), sin(lens.phase)) * 0.6;
  let n = value_noise(p * 2.2 + drift) * 0.65 + value_noise(p * 5.0 - drift * 1.7) * 0.35;
  let r = length(d) / size * (0.7 + 0.6 * n);
  let glow = exp(-r * r * 2.5);
  let hot = exp(-r * r * 9.0);
  let colour = theme();
  let warm = mix(colour, vec3f(1.0, 0.45, 0.12), 0.4);
  let light = mix(colour * 0.8, warm, hot) * glow * lens.leak;
  let burn = mix(warm, vec3f(1.0, 0.85, 0.6), 0.4) * exp(-r * r * 6.0) * lens.burn;
  return encode_srgb(min(light * 0.9 + burn * 0.8, vec3f(0.85)));
}

// How much the light leak tints the picture under it towards its colour, 0 to 1, so that a grey
// under the glow turns the colour instead of a pale peach.
fn leak_tint(uv: vec2f) -> f32 {
  let long = max(frame.aspect, 1.0);
  let d = (uv - vec2f(lens.leak_x, lens.leak_y)) * vec2f(frame.aspect, 1.0) / long;
  let r = length(d) / (lens.leak_size * (0.6 + 0.4 * lens.leak));
  return exp(-r * r * 1.5) * lens.leak;
}

// The flare's bright streak at uv, a linear amount: a thin line at the height of the brightest
// highlight, a pixel or two sharp in a soft halo, that sweeps out from the highlight to the
// sides with a hot front and then stays, fading with the glare; and the glare round the
// highlight itself.
fn flare_streak(uv: vec2f, key: vec4f) -> f32 {
  let around = length((uv - key.ba) * vec2f(frame.aspect, 1.0)) / min(frame.aspect, 1.0);
  let glare = 0.45 * exp(-around * around / 0.004) + 0.12 * exp(-around / 0.12);
  let dy = abs(uv.y - key.a) * frame.resolution.y;
  let line = exp(-dy * dy / 2.0) + 0.3 * exp(-dy / 20.0);
  let dx = abs(uv.x - key.b);
  let s = 1.0 - (1.0 - lens.sweep) * (1.0 - lens.sweep);
  let reach = s * 1.1;
  let inside = 1.0 - smoothstep(reach - 0.12, reach, dx);
  let front = exp(-pow((dx - reach + 0.06) / 0.05, 2.0)) * (1.0 - lens.sweep);
  return glare + line * inside * (0.7 * exp(-dx * 1.5) + 1.5 * front);
}

// The colour-block frame: the picture posterised to three flat tones of the theme colour (the
// shadows a deep shade, most of it the colour, the highlights a light tint), with white line
// art along the boundaries of the tones. Both come from the quarter-size picture's luma against
// the frame's mean, softened, so there are no halftone specks, and the lines are as wide
// everywhere (the distance to a boundary is the difference over the gradient). Its negative
// swaps the tones and draws the lines dark. Never full white.
fn block_frame(uv: vec2f, key: vec4f) -> vec3f {
  let size = vec2f(textureDimensions(lens_tex));
  let t = 0.75 / size;
  let mean = max(key.g, 0.005);
  var l: array<f32, 4>;
  for (var n = 0; n < 4; n++) {
    let o = vec2f(f32(n % 2) * 2.0 - 1.0, f32(n / 2) * 2.0 - 1.0) * t;
    let y = textureSampleLevel(lens_tex, smp, uv + o, 0.0).a;
    l[n] = y / (y + mean);
  }
  let value = (l[0] + l[1] + l[2] + l[3]) * 0.25;
  // The gradient per height of the frame.
  let gx = ((l[1] + l[3]) - (l[0] + l[2])) * 0.5 / (2.0 * t.x) / frame.aspect;
  let gy = ((l[2] + l[3]) - (l[0] + l[1])) * 0.5 / (2.0 * t.y);
  let slope = max(length(vec2f(gx, gy)), 1e-3);
  let pixel = 1.0 / frame.resolution.y;
  let width = 0.75 * pixel * max(1.0, frame.resolution.y / 900.0);

  let low = 0.22;
  let high = 0.6;
  let th = encode_srgb(theme());
  var deep = th * 0.55;
  var mid = th * 0.9;
  var light = mix(th, vec3f(1.0), 0.25);
  var ink = vec3f(0.92);
  if (lens.inverted > 0.5) {
    deep = mix(th, vec3f(1.0), 0.45);
    mid = mix(th, vec3f(1.0), 0.15);
    light = th * 0.4;
    ink = th * 0.2;
  }
  let to_low = (value - low) / slope;
  let to_high = (value - high) / slope;
  var c = mix(deep, mid, smoothstep(-pixel, pixel, to_low));
  c = mix(c, light, smoothstep(-pixel, pixel, to_high));
  // The line art outlines the bright shapes only; the shadows are a change of tone.
  let lines = 1.0 - smoothstep(width, width + pixel, abs(to_high));
  // A faint gradient is no edge: noise there makes no line.
  let edge = smoothstep(0.4, 1.2, slope);
  return mix(c, ink, lines * edge);
}

fn noise(coord: vec2f, salt: u32) -> vec4f {
  let s = frame.seed * 1664525u + salt * 1013904223u;
  let shift = vec2i(i32((s >> 8u) & 63u), i32((s >> 16u) & 63u));
  return textureLoad(blue_noise, (vec2i(coord) + shift) & vec2i(63), 0);
}

@fragment
fn composite(v: Varyings) -> @location(0) vec4f {
  let uv = v.uv;
  let d = (uv - 0.5) * 2.0 * post.chromatic;
  var c = vec3f(
    textureSampleLevel(source, smp, uv + d, 0.0).r,
    textureSampleLevel(source, smp, uv, 0.0).g,
    textureSampleLevel(source, smp, uv - d, 0.0).b);

  // The levels were summed with weights radius^level: divide that out.
  let r = clamp(post.bloom_radius, 0.0, 1.0);
  var total = 0.0;
  var w = 1.0;
  for (var n = 0u; n < LEVELS; n++) {
    total += w;
    w *= r;
  }
  c += textureSampleLevel(bloom, smp, uv, 0.0).rgb / max(total, 1e-4) * post.bloom_intensity;

  c *= exp2(post.exposure);
  c = c * (1.0 + 2.0 * post.flash) + vec3f(post.flash);
  c *= post.tint.rgb;

  var display: vec3f;
  if (TONEMAP == 0u) {
    display = agx(c);
  } else {
    // Without AgX the look applies in linear light.
    let l = luma(c);
    c = max(l + post.saturation * (c - l), vec3f(0.0));
    c = 0.18 * pow(c / 0.18, vec3f(post.contrast));
    if (TONEMAP == 1u) {
      display = aces(c);
    } else {
      display = encode_srgb(c);
    }
  }

  let corner = length((uv - 0.5) * vec2f(1.0, 0.8)) * 1.6;
  display *= 1.0 - post.vignette * smoothstep(0.35, 1.0, corner);
  if (lens.leak > 0.0) {
    let tint = encode_srgb(theme());
    display *= mix(vec3f(1.0), tint, leak_tint(uv) * 0.7);
    display = screen(display, leak_light(uv));
  }
  if (lens.flare > 0.0 || lens.blocks > 0.0) {
    let key = textureLoad(key_tex, vec2i(0), 0);
    if (lens.flare > 0.0) {
      let glare = textureSampleLevel(flare_tex, smp, uv, 0.0).rgb;
      let streak = flare_streak(uv, key) * mix(theme(), vec3f(1.0), 0.55);
      let light = 1.0 - exp(-(glare + streak) * lens.flare);
      display = screen(display, encode_srgb(min(light, vec3f(0.8))));
    }
    if (lens.blocks > 0.0) {
      display = mix(display, block_frame(uv, key), clamp(lens.blocks, 0.0, 1.0));
    }
  }

  let n = noise(v.position.xy, 1u);
  display += (n.r - 0.5) * post.grain * (1.2 - luma(display));
  // Dither to the 8-bit frame: no banding in dark gradients.
  display += (n.g + n.b - 1.0) / 255.0;
  return vec4f(display, 1.0);
}
