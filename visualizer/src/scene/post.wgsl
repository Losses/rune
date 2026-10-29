// The runtime's post-processing: bloom on a mip chain (13-tap downsampling with a Karis
// average on the first level, tent upsampling), then exposure, chromatic aberration, tone
// mapping, vignette, grain and dithering into the 8-bit frame. The Rust side puts the structs
// Frame and Post and the constants LEVELS and TONEMAP (0 AgX, 1 ACES, 2 none) in front.
//
// The lens effects of a scene that lists them (struct Lens, see scene/lens.rs; all zeros
// otherwise, and then nothing below costs a tap; each effect is started at a moment and runs
// its course, so these are strengths that rise and fall). The flare is drawn at a quarter of the
// size in passes of its own (`flare_bright`, `streak_down`, `streak_up`, `ghost_down`,
// `ghost_blur`, `flare`); the composite lays it and the light leak over the tone-mapped picture,
// in display values, so they lift the dark instead of greying it, and neither reaches full white.

@group(0) @binding(0) var<storage, read> post: Post;
@group(0) @binding(1) var<uniform> frame: Frame;
@group(0) @binding(2) var<uniform> lens: Lens;
@group(1) @binding(0) var source: texture_2d<f32>;
@group(1) @binding(1) var smp: sampler;
@group(1) @binding(2) var bloom: texture_2d<f32>;
@group(1) @binding(3) var blue_noise: texture_2d<f32>;
// The flare, in the composite; in `flare` the streaks' pyramid; in `flare_bright` the highlights
// of the frame before.
@group(1) @binding(4) var flare_tex: texture_2d<f32>;

// Which lens effects are on. `flare` is the flare's intensity (0 off), `stretch` how far its
// streaks reach (the weight of each wider level of their pyramid) and `history` 1 when the frame
// before drew the flare; `leak` how much of the light leak is in (0 to 1) with its source at
// (leak_x, leak_y) in uv, of radius leak_size (a share of the long side), and a phase that grows
// over the leak.
struct Lens {
  flare: f32,
  stretch: f32,
  history: f32,
  leak: f32,
  leak_x: f32,
  leak_y: f32,
  leak_size: f32,
  phase: f32,
  // How far the flare's streaks lean from the horizontal, in radians.
  angle: f32,
  pad0: f32,
  pad1: f32,
  pad2: f32,
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

// ---- The lens flare, drawn only while it is on ----
//
// After Keijiro Takahashi's Kino Streak and John Chapman's pseudo lens flare. It tracks no light
// source and adapts to nothing in the frame, so it is as steady as the picture: the highlights
// above a fixed threshold, smeared into anamorphic streaks by a pyramid that shrinks only across,
// and ghosts and a halo sampled from the highlights mirrored through the centre. All at a
// quarter of the internal width and half its height, or less.

// The threshold of the flare's highlights in the exposed HDR picture, and the width of its soft
// knee on each side, as a share of it.
const FLARE_THRESHOLD: f32 = 1.0;
const FLARE_KNEE: f32 = 0.3;
// A highlight is what stands out from its surroundings: the threshold is raised to this many
// times the brightness round it (sampled this far off, as a share of the picture's height). A
// streak is a small light smeared; a wide bright sky smeared is a white wall (Torii Sea's low sun
// washed the whole picture out so).
const FLARE_LOCAL: f32 = 1.5;
const FLARE_AROUND: f32 = 0.06;
// How much of a highlight above its threshold counts, at most: the excess is eased towards this,
// so a sun of twenty draws a streak a little longer than a lamp of three, not seven times it.
const FLARE_MOST: f32 = 4.0;

// The flare is drawn on a canvas turned by its angle (`lens.angle`), so that its streaks, which
// its pyramid smears only along the canvas's width, lean so in the picture. The canvas is turned
// about the centre and scaled up to cover the picture's corners. From a point of the canvas (its
// uv) to the picture's uv, and back.
fn flare_turn() -> vec3f {
  let c = cos(lens.angle);
  let s = sin(lens.angle);
  let a = frame.aspect;
  let cover = max(abs(c) + abs(s) / a, a * abs(s) + abs(c));
  return vec3f(c, s, cover);
}

fn flare_canvas_to_picture(q: vec2f) -> vec2f {
  let t = flare_turn();
  let a = frame.aspect;
  let p = (q - 0.5) * vec2f(a, 1.0) * t.z;
  return vec2f(p.x * t.x - p.y * t.y, p.x * t.y + p.y * t.x) / vec2f(a, 1.0) + 0.5;
}

fn flare_picture_to_canvas(uv: vec2f) -> vec2f {
  let t = flare_turn();
  let a = frame.aspect;
  let p = (uv - 0.5) * vec2f(a, 1.0);
  return vec2f(p.x * t.x + p.y * t.y, -p.x * t.y + p.y * t.x) / t.z / vec2f(a, 1.0) + 0.5;
}

// The brightness round `uv`: eight taps on a ring `FLARE_AROUND` off, the brightest channel of
// each, averaged.
fn flare_around(uv: vec2f) -> f32 {
  let r = vec2f(FLARE_AROUND / frame.aspect, FLARE_AROUND);
  var sum = 0.0;
  for (var i = 0; i < 8; i++) {
    let a = f32(i) * 0.7853982;
    let c = textureSampleLevel(source, smp, uv + vec2f(cos(a), sin(a)) * r, 0.0).rgb;
    sum += max(c.r, max(c.g, c.b));
  }
  return sum / 8.0 * exp2(post.exposure);
}

// `c`, or nothing where it is not a finite colour (a NaN or an infinity from a scene would
// otherwise be smeared across the picture and kept by the history).
fn finite(c: vec3f) -> vec3f {
  let ok = all(c == c) && all(abs(c) < vec3f(1e20));
  return select(vec3f(0.0), c, ok);
}

// The highlights at a quarter of the width and half the height: the 13 taps with the Karis
// average (no halftone dot or lone bright pixel flickers), above the threshold with a soft knee,
// and half of them those of the frame before (when it drew the flare), so a highlight that
// shimmers about the threshold glows steadily.
@fragment
fn flare_bright(v: Varyings) -> @location(0) vec4f {
  // The picture's point under this point of the turned canvas; nothing outside the picture.
  let at = flare_canvas_to_picture(v.uv);
  let inside = f32(all(at >= vec2f(0.0)) && all(at <= vec2f(1.0)));
  let c = finite(downsample(at, true) * exp2(post.exposure)) * inside;
  let b = max(c.r, max(c.g, c.b));
  let threshold = max(FLARE_THRESHOLD, FLARE_LOCAL * flare_around(at));
  let knee = threshold * FLARE_KNEE;
  var soft = clamp(b - threshold + knee, 0.0, 2.0 * knee);
  soft = soft * soft / (4.0 * knee);
  let over = max(soft, b - threshold);
  let kept = over / (1.0 + over / FLARE_MOST);
  let now = c * kept / max(b, 1e-4);
  let before = finite(textureSampleLevel(flare_tex, smp, v.uv, 0.0).rgb);
  return vec4f(mix(now, before, 0.5 * lens.history), 1.0);
}

// A level of the streaks' pyramid: the one before at half its width, the same height (two taps
// between texels, four texels).
@fragment
fn streak_down(v: Varyings) -> @location(0) vec4f {
  let dx = 1.0 / f32(textureDimensions(source).x);
  let a = textureSampleLevel(source, smp, v.uv - vec2f(dx, 0.0), 0.0).rgb;
  let b = textureSampleLevel(source, smp, v.uv + vec2f(dx, 0.0), 0.0).rgb;
  return vec4f((a + b) * 0.5, 1.0);
}

// The level below, widened back and added onto a level (blending one + one), weighted by the
// stretch: each wider level weighs that much less than the one before, so a larger stretch
// reaches further.
@fragment
fn streak_up(v: Varyings) -> @location(0) vec4f {
  let dx = 1.5 / f32(textureDimensions(source).x);
  let a = textureSampleLevel(source, smp, v.uv - vec2f(dx, 0.0), 0.0).rgb;
  let b = textureSampleLevel(source, smp, v.uv + vec2f(dx, 0.0), 0.0).rgb;
  return vec4f((a + b) * 0.5 * lens.stretch, 1.0);
}

// A ghost is the defocused image of the aperture, so every light, small or large, leaves a soft
// round disc of its own. The highlights' texels are twice as wide as tall, so reading them a
// texel off gave a small light a sharp block; the ghosts and the halo read this instead: the
// highlights averaged over a disc, measured in the picture's own units, in two passes at an
// eighth of the width and height (square texels, so the disc is round) into the other textures
// the flare keeps (`bloom` here). The first folds the highlights' texels, two by four, into
// those of the small texture over two by two of them (eight taps; each is a bilinear one that
// averages four texels or two), so a light that moves a texel changes its ghost by a fraction of
// a texel's worth.
@fragment
fn ghost_down(v: Varyings) -> @location(0) vec4f {
  let t = 1.0 / vec2f(textureDimensions(source));
  var sum = vec3f(0.0);
  for (var x = 0; x < 2; x++) {
    for (var y = 0; y < 4; y++) {
      let at = vec2f(f32(x) * 2.0 - 1.0, f32(y) * 2.0 - 3.0) * t;
      sum += textureSampleLevel(source, smp, v.uv + at, 0.0).rgb;
    }
  }
  return vec4f(sum / 8.0, 1.0);
}

// The radius of that disc, as a share of the picture's height: one texel of highlight gives a
// soft disc of about twice this across.
const GHOST_RADIUS: f32 = 0.02;

// The second pass: a mean over the disc of nineteen bilinear taps (the middle, six at half the
// radius and twelve at the full radius, turned half a step from the six). The canvas is
// `flare_turn().z` pictures high and turning does not change a disc.
@fragment
fn ghost_blur(v: Varyings) -> @location(0) vec4f {
  let r = vec2f(GHOST_RADIUS / frame.aspect, GHOST_RADIUS) / flare_turn().z;
  var sum = textureSampleLevel(source, smp, v.uv, 0.0).rgb;
  for (var i = 0; i < 6; i++) {
    let a = f32(i) * 1.0471976;
    sum += textureSampleLevel(source, smp, v.uv + vec2f(cos(a), sin(a)) * r * 0.5, 0.0).rgb;
  }
  for (var i = 0; i < 12; i++) {
    let a = (f32(i) + 0.5) * 0.5235988;
    sum += textureSampleLevel(source, smp, v.uv + vec2f(cos(a), sin(a)) * r, 0.0).rgb;
  }
  return vec4f(sum / 19.0, 1.0);
}

// The soft discs of the highlights at `at`, split a little in colour along the line from the
// centre, the more the further out (`out` is 0 at the centre, 1 in a corner), as a lens fringes.
fn bright_fringed(at: vec2f, out: f32) -> vec3f {
  let shift = normalize(at - 0.5 + vec2f(1e-5)) * 0.012 * out * out;
  let r = textureSampleLevel(bloom, smp, at + shift, 0.0).r;
  let g = textureSampleLevel(bloom, smp, at, 0.0).g;
  let b = textureSampleLevel(bloom, smp, at - shift, 0.0).b;
  return vec3f(r, g, b);
}

// How many ghosts, how far apart along the line to the centre, and the halo's radius.
const GHOSTS: i32 = 4;
const GHOST_SPACING: f32 = 0.37;
const HALO_RADIUS: f32 = 0.42;

// The flare before its intensity, in linear light (the composite eases it into display values):
// the streaks (the highlights and the pyramid's reach, in the theme colour going nearly white
// where they are bright), and Chapman's ghosts and halo, faint, faded towards the edges and
// fringed there, under a faint lens dirt that keeps them from looking sampled.
@fragment
fn flare(v: Varyings) -> @location(0) vec4f {
  let uv = v.uv;
  let colour = theme();
  let core = luma(textureSampleLevel(source, smp, uv, 0.0).rgb);
  let dx = 1.5 / f32(textureDimensions(flare_tex).x);
  let reach = luma(textureSampleLevel(flare_tex, smp, uv - vec2f(dx, 0.0), 0.0).rgb
    + textureSampleLevel(flare_tex, smp, uv + vec2f(dx, 0.0), 0.0).rgb) * 0.5 * lens.stretch;
  let dirt_at = uv * vec2f(frame.aspect, 1.0);
  let dirt = 0.7 + 0.6 * value_noise(dirt_at * 7.0) * value_noise(dirt_at * 17.0 + 3.1);

  let streak = (core * 0.6 + reach * 3.0) * mix(1.0, dirt, 0.3);
  var c = streak * mix(colour, vec3f(1.0), 0.3 + 0.55 * smoothstep(0.2, 1.5, streak));

  let flipped = 1.0 - uv;
  let towards = (0.5 - flipped) * GHOST_SPACING;
  var ghosts = vec3f(0.0);
  for (var i = 0; i < GHOSTS; i++) {
    let at = flipped + towards * f32(i);
    let out = min(length(at - 0.5) / 0.7071, 1.0);
    let fade = pow(1.0 - out, 4.0);
    ghosts += bright_fringed(at, out) * fade;
  }
  let halo_at = flipped + normalize(towards + vec2f(1e-5)) * HALO_RADIUS;
  let halo_out = min(length(halo_at - 0.5) / 0.7071, 1.0);
  let inside = smoothstep(0.0, 0.05, min(min(halo_at.x, 1.0 - halo_at.x), min(halo_at.y, 1.0 - halo_at.y)));
  let halo = bright_fringed(halo_at, halo_out) * pow(1.0 - halo_out, 5.0) * inside;
  let lens_colour = mix(colour, vec3f(1.0), 0.3);
  c += (luma(ghosts) * 0.12 + luma(halo) * 0.12) * lens_colour * dirt;
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
// colour going warmer towards the source. It grows as it comes in and shrinks as it goes.
fn leak_light(uv: vec2f) -> vec3f {
  let long = max(frame.aspect, 1.0);
  let scale = vec2f(frame.aspect, 1.0) / long;
  let p = uv * scale;
  let d = (uv - vec2f(lens.leak_x, lens.leak_y)) * scale;
  let size = lens.leak_size * (0.45 + 0.55 * lens.leak);
  let drift = vec2f(cos(lens.phase), sin(lens.phase)) * 0.6;
  let n = value_noise(p * 2.2 + drift) * 0.65 + value_noise(p * 5.0 - drift * 1.7) * 0.35;
  let r = length(d) / size * (0.7 + 0.6 * n);
  let glow = exp(-r * r * 2.5);
  let hot = exp(-r * r * 9.0);
  let colour = theme();
  let warm = mix(colour, vec3f(1.0, 0.45, 0.12), 0.4);
  let light = mix(colour * 0.8, warm, hot) * glow * lens.leak;
  return encode_srgb(min(light * 0.45, vec3f(0.85)));
}

// How much the light leak tints the picture under it towards its colour, 0 to 1, so that a grey
// under the glow turns the colour instead of a pale peach.
fn leak_tint(uv: vec2f) -> f32 {
  let long = max(frame.aspect, 1.0);
  let d = (uv - vec2f(lens.leak_x, lens.leak_y)) * vec2f(frame.aspect, 1.0) / long;
  let r = length(d) / (lens.leak_size * (0.45 + 0.55 * lens.leak));
  return exp(-r * r * 1.5) * lens.leak;
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
    display *= mix(vec3f(1.0), tint, leak_tint(uv) * 0.35);
    display = screen(display, leak_light(uv));
  }
  if (lens.flare > 0.0) {
    let flare_light = textureSampleLevel(flare_tex, smp, flare_picture_to_canvas(uv), 0.0).rgb;
    let light = 1.0 - exp(-flare_light * lens.flare);
    display = screen(display, encode_srgb(min(light, vec3f(0.8))));
  }
  let n = noise(v.position.xy, 1u);
  display += (n.r - 0.5) * post.grain * (1.2 - luma(display));
  // Dither to the 8-bit frame: no banding in dark gradients.
  display += (n.g + n.b - 1.0) / 255.0;
  return vec4f(display, 1.0);
}
