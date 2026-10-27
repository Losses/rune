// The transition pass: one fullscreen triangle that writes the output from the scene's frame
// (`fresh`) and, for the kinds that hold the old picture, the picture held from the scene before
// (`held`). `kind` picks the branch, `progress` runs from 0 to 1, `seed` makes the pattern of a
// glitch or a mosaic and `color` is for the kinds that need one (the flood of a flash, the theme
// colour of a cut-in).
//
// The glitch and the mosaic are written as functions of the held picture, the frame and the
// progress alone, so that a later filter can call them on a single picture. The cut-in
// (`cutin.wgsl` is appended to this file's source) is one more branch.

const KIND_FADE: u32 = 0u;
const KIND_DISSOLVE: u32 = 1u;
const KIND_FLASH: u32 = 2u;
const KIND_GLITCH: u32 = 3u;
const KIND_MOSAIC: u32 = 4u;
// The fade and the flash of the picture held (the way out of a dressed cut).
const KIND_FADE_HELD: u32 = 5u;
const KIND_FLASH_HELD: u32 = 6u;
const KIND_CUTIN: u32 = 7u;
// The punch on a cut of a burst, over the scene's frame alone.
const KIND_PUNCH: u32 = 8u;
// A flash's rise over the scene's frame (KIND_FLASH is its fall; KIND_FLASH_HELD the rise of a
// dressed cut).
const KIND_FLASH_RISE: u32 = 9u;

struct Transition {
  kind: u32,
  progress: f32,
  seed: u32,
  pad: u32,
  // A fade's colour, a flash's flood, the theme colour of a cut-in. Display values, like the
  // textures.
  color: vec4f,
  // A cut-in: seconds since it started, the band's opening time, its hold, its tear. A punch:
  // the side of its smear (1 or -1) in x.
  cutin: vec4f,
  // A cut-in: the style (0 speed lines and glint, 1 halftone, 2 echo), 1 if there is a cover.
  // A punch: the scale of the picture, the length of the smear (of the height), the colour
  // split (of the distance from the centre) and the flash towards `color`. A flash's rise: the
  // theme colour at its brightest (display values) and the share of the rise that is the wash.
  extra: vec4f,
}

@group(0) @binding(0) var<uniform> transition: Transition;
@group(0) @binding(1) var fresh: texture_2d<f32>;
@group(0) @binding(2) var held: texture_2d<f32>;
// The cover of the song playing, as bytes.
@group(0) @binding(3) var cover: texture_2d<f32>;
@group(0) @binding(4) var cover_sampler: sampler;

@vertex
fn vertex(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
  let xy = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
  return vec4f(xy * 2.0 - 1.0, 0.0, 1.0);
}

fn hash(x: u32) -> u32 {
  var v = x * 747796405u + 2891336453u;
  v = ((v >> ((v >> 28u) + 4u)) ^ v) * 277803737u;
  return (v >> 22u) ^ v;
}

fn hash3(a: u32, b: u32, c: u32) -> u32 {
  return hash(a ^ hash(b ^ hash(c)));
}

// A number from 0 to 1.
fn unit(h: u32) -> f32 {
  return f32(h & 0xffffffu) / 16777215.0;
}

fn luma(c: vec3f) -> f32 {
  return dot(c, vec3f(0.299, 0.587, 0.114));
}

// A pixel of `tex`, the nearest one inside the picture.
fn load(tex: texture_2d<f32>, at: vec2i) -> vec4f {
  let size = vec2i(textureDimensions(tex));
  return textureLoad(tex, clamp(at, vec2i(0), size - vec2i(1)), 0);
}

// The frame at `uv` (0 to 1 over the picture) of the held picture or the scene.
fn load_uv(tex: texture_2d<f32>, uv: vec2f) -> vec4f {
  let size = vec2f(textureDimensions(tex));
  return load(tex, vec2i(floor(uv * size)));
}

// The colour wash of a flash's rise, after the reference's colour-block frames: the picture stays
// readable, its luma running from a deep shade of the theme colour `theme` through the colour (most
// of it) to a near-white tint for the highlights; every channel is held under the flood, so the
// highlights are the flood's light tint and the rise only gets brighter from here.
fn wash(c: vec3f, theme: vec3f, flood: vec3f) -> vec3f {
  let t = clamp(luma(c), 0.0, 1.0);
  let deep = theme * 0.45;
  let light = mix(theme, vec3f(1.0), 0.85);
  let w = mix(mix(deep, theme, smoothstep(0.0, 0.5, t)), light, smoothstep(0.55, 0.95, t));
  return min(w, flood);
}

// A pixel of `tex`, `washed` (0 to 1) of the way to its colour wash in `theme` under `flood`.
fn load_washed(
  tex: texture_2d<f32>,
  at: vec2i,
  washed: f32,
  theme: vec3f,
  flood: vec3f,
) -> vec3f {
  let c = load(tex, at).rgb;
  if (washed <= 0.0) {
    return c;
  }
  return mix(c, wash(c, theme, flood), washed);
}

// A flash at `amount` (0 to 1) over `base`: the exposure rises, the picture blooms, and it goes
// over into the flood of `flood` so that at 1 nothing of the picture is left. Every channel is
// held under the flood, which is not full white: the screen never gets brighter than it. The
// picture is first `washed` of the way to its colour wash in `theme`.
fn flash(
  tex: texture_2d<f32>,
  at: vec2i,
  amount: f32,
  flood: vec3f,
  washed: f32,
  theme: vec3f,
) -> vec3f {
  let base = load_washed(tex, at, washed, theme, flood);
  // A ring of taps that grows with the amount.
  let radius = amount * 0.05 * f32(textureDimensions(tex).y);
  var bloom = vec3f(0.0);
  for (var i = 0u; i < 12u; i += 1u) {
    let angle = f32(i) * 2.399963;
    let r = radius * sqrt((f32(i) + 0.5) / 12.0);
    let tap = at + vec2i(vec2f(cos(angle), sin(angle)) * r);
    bloom += load_washed(tex, tap, washed, theme, flood);
  }
  bloom /= 12.0;
  let exposure = 1.0 + 5.0 * amount * amount;
  // The light takes the flood's hue as it rises; with no amount the picture is as it is.
  let hue = flood / max(max(flood.r, flood.g), max(flood.b, 0.001));
  var lit = (base + bloom * 1.5 * amount) * exposure * mix(vec3f(1.0), hue, amount);
  // It saturates towards the flood, which it never reaches by exposure alone, so the screen
  // never gets brighter than the flood.
  let soft = flood * (vec3f(1.0) - exp(-lit / flood));
  lit = mix(lit, soft, smoothstep(0.0, 0.5, amount));
  return mix(lit, flood, smoothstep(0.35, 1.0, amount));
}

// A flash's rise at `amount` (0 to 1): the picture washes into the theme colour at once (within
// the first few hundredths of the rise) and holds so for its first share (`extra.w`); then the
// washed picture is overexposed into the flood, which it reaches at the peak, where the switch
// or the cut happens.
fn flash_rise(tex: texture_2d<f32>, at: vec2i, amount: f32) -> vec3f {
  let share = clamp(transition.extra.w, 0.0, 0.9);
  let washed = smoothstep(0.0, 0.06, amount);
  let up = clamp((amount - share) / (1.0 - share), 0.0, 1.0);
  return flash(tex, at, up, transition.color.rgb, washed, transition.extra.rgb);
}

// The glitch: the old picture breaks into horizontal bands that shift, stretch and drag, for 3 to
// 6 steps, and the bands turn into the new picture one after another.
fn glitch(at: vec2i, progress: f32, seed: u32) -> vec3f {
  let size = vec2f(textureDimensions(fresh));
  let uv = (vec2f(at) + 0.5) / size;
  let steps = 3u + hash(seed) % 4u;
  let stage = min(u32(progress * f32(steps)), steps - 1u);
  // Bands of two scales, a coarse and a fine one, so that they are not all of one height.
  let coarse = u32(uv.y * 7.0);
  let fine = u32(uv.y * 29.0);
  let use_fine = unit(hash3(seed, coarse, 1u)) > 0.5;
  let band = select(coarse, fine + 100u, use_fine);
  // The shift of every stage adds to the one before: the picture is dragged along, as a frame
  // that feeds the next one would.
  var shift = 0.0;
  var stretch = 1.0;
  for (var j = 0u; j <= stage; j += 1u) {
    let moves = unit(hash3(seed, band, j + 10u)) < 0.6;
    let by = (unit(hash3(seed, band, j + 20u)) - 0.5) * 0.5;
    shift += select(0.0, by, moves);
    stretch += select(0.0, (unit(hash3(seed, band, j + 30u)) - 0.3) * 0.5, moves);
  }
  let x = (uv.x - 0.5) / max(stretch, 0.2) + 0.5 + shift;
  // A smear along the shift: a few taps of the held picture towards where it came from.
  var old = vec3f(0.0);
  for (var k = 0u; k < 4u; k += 1u) {
    old += load_uv(held, vec2f(x - shift * f32(k) * 0.25, uv.y)).rgb;
  }
  old /= 4.0;
  // Some bands lose their colour.
  if unit(hash3(seed, band, stage + 40u)) < 0.35 {
    old = mix(old, vec3f(luma(old)), 0.85);
  }
  // The new picture takes the bands over: later steps, more bands, and shaken a little.
  let reveal = f32(stage) / f32(steps);
  let new_at = load_uv(fresh, vec2f(uv.x + shift * 0.3, uv.y)).rgb;
  return select(old, new_at, unit(hash3(seed, band, 50u)) < reveal);
}

// The LED dot matrix of a mosaic: the cell `cell` of a grid of `pitch` pixels whose dots are
// coloured from the picture `tex` under their centres. Black between the dots.
fn dots(tex: texture_2d<f32>, at: vec2i, pitch: f32) -> vec3f {
  let local = vec2f(at) / pitch;
  let centre = (floor(local) + 0.5) * pitch;
  let under = load(tex, vec2i(centre)).rgb;
  // A bright dot grows with the picture's brightness; the colour is brought up so that a dot of
  // a dark picture still glows.
  let size = 0.22 + 0.26 * sqrt(clamp(luma(under) * 1.6, 0.0, 1.0));
  let away = length(fract(local) - 0.5);
  let edge = 0.06;
  let lit = 1.0 - smoothstep(size - edge, size + edge, away);
  return min(under * 1.35, vec3f(1.0)) * lit;
}

// The mosaic. In the first part rectangles of a coarse grid turn into dots, a new batch every
// 1 to 3 frames, or, in some, row after row; then the whole screen is dots, then it is black for a
// frame or two, then the new picture.
fn mosaic(at: vec2i, progress: f32, seed: u32) -> vec3f {
  let size = vec2f(textureDimensions(fresh));
  let cell = size.y / 7.0;
  let pitch = cell / 6.0;
  if progress >= 0.89 {
    return load(fresh, at).rgb;
  }
  if progress >= 0.84 {
    return vec3f(0.0);
  }
  if progress >= 0.62 {
    return dots(held, at, pitch);
  }
  // Rectangles of 2 or 3 cells by 1 or 2.
  let rect = vec2u(vec2f(at) / cell) / vec2u(2u + hash(seed) % 2u, 1u + hash(seed + 1u) % 2u);
  let batches = 10u;
  let stage = u32(progress / 0.62 * f32(batches));
  let by_rows = hash(seed + 2u) % 3u == 0u;
  var appears: u32;
  if by_rows {
    // Row after row, from the top or the bottom, with a little disorder.
    let rows = u32(ceil(size.y / cell / 1.0)) / (1u + hash(seed + 1u) % 2u) + 1u;
    let row = select(rect.y, rows - 1u - min(rect.y, rows - 1u), hash(seed + 3u) % 2u == 0u);
    appears = min(row * batches / rows + hash3(seed, rect.x, rect.y) % 2u, batches);
  } else {
    appears = hash3(seed, rect.x, rect.y) % batches;
  }
  if stage >= appears {
    return dots(held, at, pitch);
  }
  return load(held, at).rgb;
}

// The punch on a cut of a burst: the frame scaled about its centre, smeared along a diagonal
// (a few taps trailing back from each pixel), its colour channels split outwards from the
// centre, and lightly flooded with `color`. All of it fades to nothing as the values go to
// 0 and the scale to 1, which is the frame as it is.
fn punch(at: vec2i) -> vec3f {
  let size = vec2f(textureDimensions(fresh));
  let uv = (vec2f(at) + 0.5) / size;
  let zoom = transition.extra.x;
  let split = transition.extra.z;
  // The smear's reach in uv: along a diagonal in pixels (y is down), `extra.y` of the height.
  let diagonal = normalize(vec2f(transition.cutin.x, -1.0));
  let step = diagonal * vec2f(size.y / size.x, 1.0) * transition.extra.y;
  let taps = 6u;
  var rgb = vec3f(0.0);
  for (var k = 0u; k < taps; k += 1u) {
    let back = step * (f32(k) / f32(taps - 1u));
    let base = (uv - 0.5) / zoom + 0.5 - back;
    let centred = base - 0.5;
    rgb.r += load_uv(fresh, 0.5 + centred * (1.0 + split)).r;
    rgb.g += load_uv(fresh, base).g;
    rgb.b += load_uv(fresh, 0.5 + centred * (1.0 - split)).b;
  }
  rgb /= f32(taps);
  return mix(rgb, transition.color.rgb, transition.extra.w);
}

// The frame as it is in the output's bytes: the textures are the output's format and size, so
// each pixel is read as it is, and blended as the bytes are (a fade through black is a fade of
// the displayed values).
@fragment
fn fragment(@builtin(position) position: vec4f) -> @location(0) vec4f {
  let at = vec2i(position.xy);
  let frame_now = textureLoad(fresh, at, 0);
  switch transition.kind {
    // `progress` of the way from the frame to `color`.
    case KIND_FADE: {
      return vec4f(mix(frame_now.rgb, transition.color.rgb, transition.progress), frame_now.a);
    }
    // From the held picture (0) to the frame (1).
    case KIND_DISSOLVE: {
      let old = textureLoad(held, at, 0);
      return mix(old, frame_now, transition.progress);
    }
    case KIND_CUTIN: {
      return cutin(position.xy);
    }
    case KIND_PUNCH: {
      return vec4f(punch(at), frame_now.a);
    }
    case KIND_FLASH_RISE: {
      return vec4f(flash_rise(fresh, at, transition.progress), frame_now.a);
    }
    case KIND_FLASH: {
      let none = vec3f(0.0);
      return vec4f(flash(fresh, at, transition.progress, transition.color.rgb, 0.0, none), frame_now.a);
    }
    case KIND_GLITCH: {
      return vec4f(glitch(at, transition.progress, transition.seed), frame_now.a);
    }
    case KIND_MOSAIC: {
      return vec4f(mosaic(at, transition.progress, transition.seed), frame_now.a);
    }
    case KIND_FADE_HELD: {
      let old = textureLoad(held, at, 0);
      return vec4f(mix(old.rgb, transition.color.rgb, transition.progress), old.a);
    }
    case KIND_FLASH_HELD: {
      let old = textureLoad(held, at, 0);
      return vec4f(flash_rise(held, at, transition.progress), old.a);
    }
    default: {
      return frame_now;
    }
  }
}
