// The runtime's post-processing: bloom on a mip chain (13-tap downsampling with a Karis
// average on the first level, tent upsampling), then exposure, chromatic aberration, tone
// mapping, vignette, grain and dithering into the 8-bit frame. The Rust side puts the structs
// Frame and Post and the constants LEVELS and TONEMAP (0 AgX, 1 ACES, 2 none) in front.
//
// The lens effects of a scene that lists them (struct Lens, see scene/lens.rs; all zeros
// otherwise, and then nothing below costs a tap; each effect is started at a moment and runs
// its course, so these are strengths that rise and fall): a flare made from the bloom in a pass of its
// own (`flare`), a light leak and a colour-block frame in the composite.

@group(0) @binding(0) var<storage, read> post: Post;
@group(0) @binding(1) var<uniform> frame: Frame;
@group(0) @binding(2) var<uniform> lens: Lens;
@group(1) @binding(0) var source: texture_2d<f32>;
@group(1) @binding(1) var smp: sampler;
@group(1) @binding(2) var bloom: texture_2d<f32>;
@group(1) @binding(3) var blue_noise: texture_2d<f32>;
@group(1) @binding(4) var flare_tex: texture_2d<f32>;

// Which lens effects are on. `flare` is the strength of the burst (0 off), `leak` how much of the light leak
// is in (0 to 1) with its source at (leak_x, leak_y) in uv, of radius leak_size (in heights) and a
// phase that grows over the leak; `blocks` is 1 on a colour-block frame.
struct Lens {
  flare: f32,
  leak: f32,
  blocks: f32,
  phase: f32,
  leak_x: f32,
  leak_y: f32,
  leak_size: f32,
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

// What of a bloom value throws the flare: only the bright part, softly.
fn flare_bright(c: vec3f) -> vec3f {
  let l = luma(c);
  return c * max(l - 0.3, 0.0) / max(l, 1e-4);
}

fn flare_tap(uv: vec2f) -> vec3f {
  return flare_bright(textureSampleLevel(source, smp, uv, 0.0).rgb);
}

// A pseudo lens flare from a bloom level, as John Chapman's: no light sources are looked for.
// The bright parts throw a thin horizontal streak (a gather along the row, as an anamorphic lens
// stretches them), a few soft ghosts (the picture sampled again along the line through the
// centre, fading towards the edge) and a halo ring. All of it is in the theme's colour. Drawn
// into a texture the size of the level, so a pixel of it is worth sixteen of the picture.
@fragment
fn flare(v: Varyings) -> @location(0) vec4f {
  let uv = v.uv;
  let tint = mix(vec3f(1.0), theme(), 0.7);

  var streak = vec3f(0.0);
  var total = 0.0;
  for (var i = -10; i <= 10; i++) {
    let x = f32(i) / 10.0;
    let w = exp(-3.0 * abs(x));
    streak += flare_tap(uv + vec2f(x * 0.4, 0.0)) * w;
    total += w;
  }
  streak = streak / total * 8.0;

  let to_centre = vec2f(0.5) - uv;
  var ghosts = vec3f(0.0);
  for (var i = 1; i <= 3; i++) {
    let at = uv + to_centre * 0.55 * f32(i);
    let fade = clamp(1.0 - length(vec2f(0.5) - at) / 0.7071, 0.0, 1.0);
    ghosts += flare_tap(at) * (fade * fade * fade) * (1.0 / f32(i));
  }

  let dir = normalize(to_centre + vec2f(1e-5)) * 0.42;
  let ring = clamp(1.0 - length(vec2f(0.5) - (uv + dir)) / 0.7071, 0.0, 1.0);
  let halo = flare_tap(uv + dir) * pow(ring, 5.0);

  return vec4f((streak + ghosts * 1.0 + halo * 1.0) * tint, 1.0);
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

// The light of the leak at uv, in the theme's colour a little warmer: a soft blob round its
// source, whose edge breathes with the phase. Added in linear light, peaking well under white.
fn leak_light(uv: vec2f) -> vec3f {
  let d = (uv - vec2f(lens.leak_x, lens.leak_y)) * vec2f(frame.aspect, 1.0);
  let r = length(d) / lens.leak_size;
  let lobes = 1.0 + 0.18 * sin(atan2(d.y, d.x) * 3.0 + lens.phase) + 0.1 * sin(r * 4.0 - lens.phase * 2.0);
  let fall = pow(clamp(1.0 - r * lobes, 0.0, 1.0), 2.0);
  let colour = mix(theme(), vec3f(1.0, 0.7, 0.4), 0.2);
  return colour * fall * 0.45 * lens.leak;
}

// The colour-block frame: the whole screen flat in the theme's colour, and the picture's bright
// edges over it as white line art. Peaks are capped, so it is never a flash to full white.
fn block_frame(uv: vec2f) -> vec3f {
  let texel = 1.5 / vec2f(textureDimensions(source));
  let ex = exp2(post.exposure);
  var l: array<f32, 9>;
  for (var n = 0; n < 9; n++) {
    let o = vec2f(f32(n % 3) - 1.0, f32(n / 3) - 1.0) * texel;
    let c = textureSampleLevel(source, smp, uv + o, 0.0).rgb * ex;
    let y = luma(c);
    l[n] = y / (1.0 + y);
  }
  let gx = (l[2] + 2.0 * l[5] + l[8]) - (l[0] + 2.0 * l[3] + l[6]);
  let gy = (l[6] + 2.0 * l[7] + l[8]) - (l[0] + 2.0 * l[1] + l[2]);
  let edge = smoothstep(0.03, 0.12, length(vec2f(gx, gy)));
  let bright = smoothstep(0.02, 0.15, max(l[4], max(l[1], max(l[3], max(l[5], l[7])))));
  let fill = encode_srgb(theme()) * 0.8;
  return mix(fill, vec3f(0.92), edge * bright);
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
  if (lens.flare > 0.0) {
    c += textureSampleLevel(flare_tex, smp, uv, 0.0).rgb * lens.flare;
  }
  if (lens.leak > 0.0) {
    c += leak_light(uv);
  }

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
  if (lens.blocks > 0.0) {
    display = mix(display, block_frame(uv), clamp(lens.blocks, 0.0, 1.0));
  }

  let n = noise(v.position.xy, 1u);
  display += (n.r - 0.5) * post.grain * (1.2 - luma(display));
  // Dither to the 8-bit frame: no banding in dark gradients.
  display += (n.g + n.b - 1.0) / 255.0;
  return vec4f(display, 1.0);
}
