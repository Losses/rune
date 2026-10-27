// The transition pass: one fullscreen triangle that writes the output from the scene's frame
// (`fresh`) and, for the kinds that hold the old picture, the picture held from the scene before
// (`held`). `kind` picks the branch, `progress` runs from 0 to 1, `seed` makes the pattern of a
// glitch or a mosaic and `color` is for the kinds that need one (the flood of a flash, the theme
// colour of a cut-in, a glitch or a mosaic).
//
// The glitch and the mosaic are written as functions of an old and a new picture and the step
// they are at (worked out in `transition.rs`), so that the same functions draw them as lens
// effects over the live frame alone (the two pictures the same). The cut-in (`cutin.wgsl` is
// appended to this file's source) is one more branch.

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

struct Transition {
  kind: u32,
  progress: f32,
  seed: u32,
  pad: u32,
  // A fade's colour, a flash's flood, the theme colour of a cut-in, a glitch or a mosaic.
  // Display values, like the textures.
  color: vec4f,
  // A cut-in: seconds since it started, the band's opening time, its hold, its tear. A punch:
  // the side of its smear (1 or -1) in x.
  cutin: vec4f,
  // A cut-in: the style (0 speed lines and glint, 1 halftone, 2 echo), 1 if there is a cover,
  // 1 if it tears (the frame is the new scene's from then on, the old one's before).
  // A punch: the scale of the picture, the length of the smear (of the height), the colour
  // split (of the distance from the centre) and the flash towards `color`.
  // A glitch: the share of pieces that break, the share that show the new picture, 1 for the
  // subtle one over the picture. A mosaic:
  // the mode, the pattern or look, the density, 1 over the new picture (see `mosaic`).
  extra: vec4f,
}

@group(0) @binding(0) var<uniform> transition: Transition;
@group(0) @binding(1) var fresh: texture_2d<f32>;
@group(0) @binding(2) var held: texture_2d<f32>;
// The cover of the song playing, as bytes, for a cut-in.
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

// A flash at `amount` (0 to 1) over `base`: the exposure rises, the picture blooms, and it goes
// over into the flood of `flood` so that at 1 nothing of the picture is left. Every channel is
// held under the flood, which is not full white: the screen never gets brighter than it.
fn flash(tex: texture_2d<f32>, at: vec2i, amount: f32, flood: vec3f) -> vec3f {
  let base = load(tex, at).rgb;
  // A ring of taps that grows with the amount.
  let radius = amount * 0.05 * f32(textureDimensions(tex).y);
  var bloom = vec3f(0.0);
  for (var i = 0u; i < 12u; i += 1u) {
    let angle = f32(i) * 2.399963;
    let r = radius * sqrt((f32(i) + 0.5) / 12.0);
    bloom += load(tex, at + vec2i(vec2f(cos(angle), sin(angle)) * r)).rgb;
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

// The picture at `uv` of the new picture (the scene's frame) or the old one (the picture held).
// Textures cannot be picked as values, so the choice is a branch.
fn either(before: texture_2d<f32>, after: texture_2d<f32>, use_new: bool, uv: vec2f) -> vec3f {
  if use_new {
    return load_uv(after, uv).rgb;
  }
  return load_uv(before, uv).rgb;
}

// The theme colour at its brightest: the hue, with its largest channel at 1.
fn hue_of(theme: vec3f) -> vec3f {
  return theme / max(max(theme.r, theme.g), max(theme.b, 0.001));
}

// One broken piece of a glitch at `uv` (see `glitch`): it shifts along its row with a smear,
// streaks (one line of the picture drawn down the row), goes blocky, flares up in light of white
// and the theme colour, or turns duotone in the theme colour. `id` makes which and how much; `top`
// and `rows` are the row's top and the rows the frame has.
fn glitch_piece(
  before: texture_2d<f32>,
  after: texture_2d<f32>,
  at: vec2i,
  uv: vec2f,
  id: u32,
  use_new: bool,
  top: f32,
  rows: f32,
  theme: vec3f,
) -> vec3f {
  let what = unit(hash(id ^ 0x9u));
  let shift = (unit(hash(id ^ 0x21u)) - 0.5) * 0.5;
  let hue = hue_of(theme);
  // Barcode stripes: columns of 1 to 4 pixels at levels of their own, which the broken pieces
  // carry so that they read as broken data even over a dark picture.
  let column = u32(f32(at.x) / f32(1u + hash(id ^ 0x61u) % 4u));
  let barcode = unit(hash3(id, column, 0x62u));
  if what < 0.34 {
    // Shifted along the row, smeared back towards where it came from, a little brighter and
    // with fine lines across it.
    var rgb = vec3f(0.0);
    for (var k = 0u; k < 4u; k += 1u) {
      rgb += either(before, after, use_new, vec2f(uv.x - shift * (1.0 + f32(k) * 0.08), uv.y));
    }
    let lines = select(1.0, 0.7, (at.y & 1) == 1);
    return min(rgb / 4.0 * 1.3 * lines, vec3f(0.9));
  }
  if what < 0.56 {
    // A streak: one line of the picture drawn down the whole row in vertical stripes, lit up,
    // over barcode stripes of the theme colour.
    let line = top + unit(hash(id ^ 0x33u)) / rows;
    let streak = either(before, after, use_new, vec2f(uv.x - shift * 0.3, line));
    let lit = streak * 1.7 + hue * 0.35 * barcode * barcode;
    return min(lit, vec3f(0.9));
  }
  if what < 0.72 {
    // Blocky: the picture in wide pixels along the row.
    let q = 0.004 + 0.03 * unit(hash(id ^ 0x45u));
    return either(before, after, use_new, vec2f((floor(uv.x / q) + 0.5) * q - shift * 0.2, uv.y));
  }
  let under = either(before, after, use_new, vec2f(uv.x - shift * 0.15, uv.y));
  let l = luma(under);
  if what < 0.9 {
    // A slab of light, white leaning to the theme colour, in barcode stripes, the picture
    // faint in it. Never full white.
    let light = mix(vec3f(1.0), hue, 0.2 + 0.5 * unit(hash(id ^ 0x57u))) * 0.86;
    return light * (0.62 + 0.22 * barcode + 0.16 * l);
  }
  // Duotone in four steps: black, the theme colour, white.
  let steps = floor(l * 4.0) / 3.0;
  let low = hue * 0.55 * clamp(steps * 2.0, 0.0, 1.0);
  return mix(low, vec3f(0.85), clamp(steps * 2.0 - 1.0, 0.0, 1.0));
}

// The glitch, one stage of it (its step is worked out in `transition.rs`): the frame is cut into
// rows of three heights, and each row into pieces from a twentieth of the width to most of it. A
// share `amount` of the pieces breaks (see `glitch_piece`). A share `reveal` of the pieces shows
// the new picture (`after`). `subtle` is the one over the live picture (`before` and `after` the
// same texture): thin rows only, narrower pieces, and each broken piece half see-through.
fn glitch(
  before: texture_2d<f32>,
  after: texture_2d<f32>,
  at: vec2i,
  pattern: u32,
  amount: f32,
  reveal: f32,
  subtle: bool,
  theme: vec3f,
) -> vec3f {
  let size = vec2f(textureDimensions(after));
  let uv = (vec2f(at) + 0.5) / size;
  // Coarse rows, each of one height of row: a tenth of the frame, a 28th or an 80th; subtle, a
  // 45th or a 90th.
  let coarse = u32(uv.y * 10.0);
  let pick = unit(hash3(pattern, coarse, 1u));
  var rows = 10.0;
  if pick > 0.4 {
    rows = 28.0;
  }
  if pick > 0.75 {
    rows = 80.0;
  }
  if subtle {
    rows = select(45.0, 90.0, pick > 0.5);
  }
  let row = u32(uv.y * rows);
  let band = hash3(pattern, row, u32(rows));
  let top = f32(row) / rows;
  // The pieces of the row: narrow ones more often than wide ones.
  let w = unit(hash(band ^ 0x51u));
  let width = select(0.06 + 0.64 * w * w, 0.05 + 0.3 * w * w, subtle);
  let piece = u32(floor(uv.x / width + unit(hash(band ^ 0x77u)) + 8.0));
  let id = hash3(band, piece, 3u);
  let use_new = unit(hash(id ^ 0x1234u)) < reveal;
  let plain = either(before, after, use_new, uv);
  if unit(hash(id)) >= amount {
    return plain;
  }
  let broken = glitch_piece(before, after, at, uv, id, use_new, top, rows, theme);
  return select(broken, mix(plain, broken, 0.6), subtle);
}

// The pitch of a mosaic's LED dots in pixels: 54 dots down the frame (13 pixels at 720p, 20 at
// 1080p, as in the reference), whole pixels so that every dot is the same.
fn led_pitch(size: vec2f) -> f32 {
  return max(round(size.y / 54.0), 4.0);
}

// How much of an LED dot covers the pixel at `d` (in dots): the dots are rounded rectangles of
// `half` the pitch each way from their centre, taller than wide so that they run into columns,
// with dark gaps, soft over a pixel.
fn led_dot(d: vec2f, pitch: f32, half: vec2f) -> f32 {
  let local = fract(d) - 0.5;
  let r = 0.12;
  let q = abs(local) - (half - r);
  let dist = length(max(q, vec2f(0.0))) + min(max(q.x, q.y), 0.0) - r;
  return 1.0 - smoothstep(-0.5 / pitch, 0.5 / pitch, dist);
}

// The colour of lit LED dots: white leaning to the theme colour by `lean`, never full white.
fn led_tint(theme: vec3f, lean: f32) -> vec3f {
  return mix(vec3f(1.0), hue_of(theme), lean) * 0.92;
}

// The size of the dots of a block.
const BLOCK_DOT: vec2f = vec2f(0.32, 0.46);
// How strongly a block's dots light up over the picture, and how much its gaps darken it, at its
// fullest: a faint veil, not a panel.
const VEIL_LIGHT: f32 = 0.24;
const VEIL_PICTURE: f32 = 0.35;
const VEIL_GAP: f32 = 0.16;

// How bright each dot of a block is: most near full, a few dimmer, one in fifty out, as on a real
// panel.
fn dot_level(seed: u32, n: vec2f) -> f32 {
  let h = unit(hash3(seed ^ 0x5eedu, u32(n.x + 4096.0), u32(n.y + 4096.0)));
  return select(0.85 + 0.15 * h / 0.98, 0.0, h > 0.98);
}

// The block of a batch at dot `n`: its rectangle in dots (x, y, x + w, y + h; empty when there
// is none), how strong its veil is, its lean to the theme colour, and whether only every other
// column of its dots is on.
struct Block {
  rect: vec4f,
  level: f32,
  lean: f32,
  sparse: bool,
}

// The batch of blocks of `pattern` (0 scatter, 1 checker, 2 stagger) and `density` that covers
// dot `n`. They come in small groups apart from each other: cells of 8 by 6 dots, regions of 3 by
// 2 cells (placed by the seed, not on the frame's edges) of which only every other one, as on a
// chessboard, may hold a group, so that no two groups touch; a share `density` of those lit, and
// in a lit region a few cells with a small block each, aligned to the dots.
fn block_at(n: vec2f, seed: u32, pattern: u32, density: f32) -> Block {
  var block = Block(vec4f(0.0), 0.0, 0.0, false);
  let shift = vec2f(f32(hash(seed ^ 0x0ffu) % 24u), f32(hash(seed ^ 0x0feu) % 18u));
  let m = n + shift;
  let cy = floor(m.y / 6.0);
  var offset = 0.0;
  if pattern == 2u && (u32(cy) & 1u) == 1u {
    offset = 4.0;
  }
  let cx = floor((m.x + offset) / 8.0);
  let rx = u32(floor(cx / 3.0) + 512.0);
  let ry = u32(floor(cy / 2.0) + 512.0);
  if ((rx + ry) & 1u) == 1u || unit(hash3(seed ^ 0x7e9u, rx, ry)) >= density {
    return block;
  }
  let cell = hash3(seed, u32(cx + 512.0), u32(cy + 512.0));
  let roll = unit(cell);
  let origin = vec2f(cx * 8.0 - offset, cy * 6.0) - shift;
  var lit = false;
  var size = vec2f(5.0, 3.0);
  var at = vec2f(2.0, 2.0);
  switch pattern {
    case 1u: {
      // Every other cell of the region.
      lit = (u32(cx + cy + 1024.0) & 1u) == 0u && roll < 0.85;
      size = vec2f(6.0, 4.0);
      at = vec2f(1.0, 1.0);
    }
    case 2u: {
      lit = roll < 0.65;
      at = vec2f(1.0, 2.0);
    }
    default: {
      // Blocks of 3 to 6 dots by 2 to 4, anywhere in their cell.
      lit = roll < 0.5;
      size = vec2f(f32(3u + hash(cell ^ 0x1u) % 4u), f32(2u + hash(cell ^ 0x2u) % 3u));
      at = vec2f(
        f32(hash(cell ^ 0x3u) % u32(9.0 - size.x)),
        f32(hash(cell ^ 0x4u) % u32(7.0 - size.y)),
      );
    }
  }
  if lit {
    block.rect = vec4f(origin + at, origin + at + size);
    // Some with every other column of dots only, as panels switching on; some fainter.
    block.sparse = unit(hash(cell ^ 0x5u)) < 0.2;
    block.level = 0.6 + 0.4 * unit(hash(cell ^ 0x6u));
    block.lean = 0.25 + 0.5 * unit(hash(cell ^ 0x7u));
  }
  return block;
}

// A batch of faint LED blocks over `under` (the picture at the pixel): in a block, dots that
// carry the picture a little brighter with a touch of light, the gaps a little darker. A texture
// over the picture, which shows through.
fn blocks_over(
  under: vec3f,
  at: vec2i,
  size: vec2f,
  seed: u32,
  pattern: u32,
  density: f32,
  theme: vec3f,
) -> vec3f {
  let pitch = led_pitch(size);
  let d = (vec2f(at) + 0.5) / pitch;
  let n = floor(d);
  let block = block_at(n, seed, pattern, density);
  if block.level <= 0.0 || !(all(d >= block.rect.xy) && all(d < block.rect.zw)) {
    return under;
  }
  var dot = led_dot(d, pitch, BLOCK_DOT) * dot_level(seed, n);
  if block.sparse && (u32(n.x) & 1u) == 1u {
    dot = 0.0;
  }
  let light = led_tint(theme, block.lean) * VEIL_LIGHT + under * VEIL_PICTURE;
  return min(under * (1.0 - VEIL_GAP * block.level) + light * dot * block.level, vec3f(0.92));
}

// The whole screen as LED dots of `look` (0 the picture dimmed under a veil of dots, 1 a lighter
// veil, 2 the same with some bands gone), in vertical bands of 3 to 11 dots, each a little
// different. The dots carry the picture under their centres, so it is in dots too; the gaps show
// it dimmed. Soft: never brighter than the picture much.
fn led_screen(tex: texture_2d<f32>, at: vec2i, seed: u32, look: u32, theme: vec3f) -> vec3f {
  let size = vec2f(textureDimensions(tex));
  let pitch = led_pitch(size);
  let d = (vec2f(at) + 0.5) / pitch;
  let n = floor(d);
  // Bands: every 14 dots split once at a place of their own.
  let big = floor(n.x / 14.0);
  let split = f32(3u + hash3(seed, u32(big + 512.0), 9u) % 8u);
  let band = hash3(seed, u32(big + 512.0), select(1u, 2u, n.x - big * 14.0 >= split));
  let tone = unit(band);
  let here = load(tex, at).rgb;
  if look == 2u && tone < 0.35 {
    return here;
  }
  let dotted = load(tex, vec2i((n + 0.5) * pitch)).rgb;
  let dot = led_dot(d, pitch, BLOCK_DOT);
  let tint = led_tint(theme, 0.4 + 0.5 * tone);
  if look == 0u {
    // The moment that hides the cut: the picture at half, the dots over it.
    let lit = dotted * (0.75 + 0.15 * tone) + tint * 0.12;
    return mix(here * 0.5, lit, dot * 0.8);
  }
  let lit = dotted * (0.85 + 0.15 * tone) + tint * 0.06;
  return mix(here * 0.78, lit, dot * 0.6);
}

// The mosaic, one pose of it (its step is worked out in `transition.rs`): `mode` 1 is a batch of
// blocks over the picture, 2 the whole screen as dots; `kind` is the pattern of the
// blocks or the look of the dots; `use_new` picks the new picture (the scene's frame) or the old
// one (the picture held). With `before` and `after` the same texture it is the lens effect.
fn mosaic(
  before: texture_2d<f32>,
  after: texture_2d<f32>,
  at: vec2i,
  seed: u32,
  mode: u32,
  kind: u32,
  density: f32,
  use_new: bool,
  theme: vec3f,
) -> vec3f {
  let size = vec2f(textureDimensions(after));
  if mode == 2u {
    if use_new {
      return led_screen(after, at, seed, kind, theme);
    }
    return led_screen(before, at, seed, kind, theme);
  }
  var under: vec3f;
  if use_new {
    under = load(after, at).rgb;
  } else {
    under = load(before, at).rgb;
  }
  return blocks_over(under, at, size, seed, kind, density, theme);
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
    case KIND_FLASH: {
      return vec4f(flash(fresh, at, transition.progress, transition.color.rgb), frame_now.a);
    }
    case KIND_GLITCH: {
      let e = transition.extra;
      let subtle = e.z > 0.5;
      let rgb = glitch(held, fresh, at, transition.seed, e.x, e.y, subtle, transition.color.rgb);
      return vec4f(rgb, frame_now.a);
    }
    case KIND_MOSAIC: {
      let e = transition.extra;
      let rgb = mosaic(
        held,
        fresh,
        at,
        transition.seed,
        u32(e.x),
        u32(e.y),
        e.z,
        e.w > 0.5,
        transition.color.rgb,
      );
      return vec4f(rgb, frame_now.a);
    }
    case KIND_FADE_HELD: {
      let old = textureLoad(held, at, 0);
      return vec4f(mix(old.rgb, transition.color.rgb, transition.progress), old.a);
    }
    case KIND_FLASH_HELD: {
      let old = textureLoad(held, at, 0);
      return vec4f(flash(held, at, transition.progress, transition.color.rgb), old.a);
    }
    default: {
      return frame_now;
    }
  }
}
