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
// A flash's rise over the scene's frame (KIND_FLASH is its fall; KIND_FLASH_HELD the rise of a
// dressed cut).
const KIND_FLASH_RISE: u32 = 9u;

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
  // subtle one over the picture; over the picture as a lens effect: its amount, its time in
  // beats, 1, and 1 (see `glitch_flow`; the beat's accent is in the progress). A mosaic: 0 for a
  // switch (its course is the progress); over the picture 1, its amount and its eased beat time
  // (see `mosaic_blocks`; the beat's accent is in the progress). A flash's rise:
  // the theme colour at its brightest (display values) and the share of the rise that is the
  // wash.
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
  let shift = (unit(hash(id ^ 0x21u)) - 0.5) * 0.7;
  let hue = hue_of(theme);
  // Barcode stripes: columns of 1 to 4 pixels at levels of their own, which the broken pieces
  // carry so that they read as broken data even over a dark picture.
  let column = u32(f32(at.x) / f32(1u + hash(id ^ 0x61u) % 4u));
  let barcode = unit(hash3(id, column, 0x62u));
  if what < 0.3 {
    // Shifted along the row, its colour channels torn apart, smeared back towards where it
    // came from, with fine lines across it.
    let split = 0.01 + 0.03 * unit(hash(id ^ 0x71u));
    var rgb = vec3f(0.0);
    for (var k = 0u; k < 4u; k += 1u) {
      let x = uv.x - shift * (1.0 + f32(k) * 0.08);
      rgb.r += either(before, after, use_new, vec2f(x + split, uv.y)).r;
      rgb.g += either(before, after, use_new, vec2f(x, uv.y)).g;
      rgb.b += either(before, after, use_new, vec2f(x - split, uv.y)).b;
    }
    let lines = select(1.0, 0.8, (at.y & 1) == 1);
    return min(rgb / 4.0 * 1.2 * lines, vec3f(0.9));
  }
  if what < 0.42 {
    // Shifted and inverted.
    let x = uv.x - shift;
    let inverted = vec3f(1.0) - either(before, after, use_new, vec2f(x, uv.y));
    return min(inverted, vec3f(0.9));
  }
  if what < 0.58 {
    // A streak: one line of the picture drawn down the whole row in vertical stripes, lit up,
    // over barcode stripes of the theme colour.
    let line = top + unit(hash(id ^ 0x33u)) / rows;
    let streak = either(before, after, use_new, vec2f(uv.x - shift * 0.3, line));
    let lit = streak * 1.7 + hue * 0.35 * barcode * barcode;
    return min(lit, vec3f(0.9));
  }
  if what < 0.7 {
    // Blocky: the picture in wide pixels along the row.
    let q = 0.004 + 0.03 * unit(hash(id ^ 0x45u));
    return either(before, after, use_new, vec2f((floor(uv.x / q) + 0.5) * q - shift * 0.2, uv.y));
  }
  let under = either(before, after, use_new, vec2f(uv.x - shift * 0.15, uv.y));
  let l = luma(under);
  if what < 0.88 {
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

// Smooth noise in one dimension, 0 to 1: a value at each whole number, eased between.
fn noise1(x: f32, salt: u32) -> f32 {
  let i = floor(x);
  let f = x - i;
  let a = unit(hash3(salt, u32(i + 4096.0), 0x3du));
  let b = unit(hash3(salt, u32(i + 4097.0), 0x3du));
  return mix(a, b, f * f * (3.0 - 2.0 * f));
}

// The glitch as a lens effect over the live picture `tex`, on the eased beat time `t`: the picture
// jumps. Each beat is cut into three states, the eased time passing them in its first fifth, so
// every beat the tears jump to a new layout and hold it; now and then (a fifth of the beats) a
// beat holds one state all through, with its rows held (a stutter). The frame is cut into rows
// of one of three heights (a tenth, a 24th, a 60th of the height), and `amount` (0 to 1) decides
// how many are torn: up to 40%. A torn row, whole or from one edge, is the picture shifted
// sideways by a tenth of the width or more (the beat's `accent`, 0 to 1, pushes it further), its
// colour channels torn apart, and some rows are inverted or burnt to the theme colour. Never
// full white.
fn glitch_flow(
  tex: texture_2d<f32>,
  at: vec2i,
  seed: u32,
  amount: f32,
  t: f32,
  accent: f32,
  theme: vec3f,
) -> vec3f {
  let size = vec2f(textureDimensions(tex));
  let uv = (vec2f(at) + 0.5) / size;
  let here = load(tex, at).rgb;
  let beat = floor(t);
  let sub = min(u32((t - beat) * 3.0), 2u);
  let stutter = unit(hash3(seed, u32(beat), 0xc1u)) < 0.2;
  let state = hash3(seed ^ 0x5bd1u, u32(beat), select(sub, 3u, stutter));
  let coarse = u32(uv.y * 8.0);
  let pick = unit(hash3(state, coarse, 1u));
  var rows = 10.0;
  if pick > 0.3 {
    rows = 24.0;
  }
  if pick > 0.7 {
    rows = 60.0;
  }
  let row = u32(uv.y * rows);
  let band = hash3(state, row, u32(rows));
  let strength = clamp(amount, 0.0, 1.0);
  if unit(hash(band ^ 0x11u)) >= 0.4 * strength {
    return here;
  }
  // How far along the row the tear reaches, from the left or the right edge.
  let reach = select(1.0, 0.4 + 0.6 * unit(hash(band ^ 0x13u)), unit(hash(band ^ 0x12u)) < 0.5);
  let from_left = (band & 0x100u) == 0u;
  let outside = select((uv.x < 1.0 - reach), (uv.x > reach), from_left);
  if outside {
    return here;
  }
  let sign = select(-1.0, 1.0, (band & 0x200u) == 0u);
  let off = sign * ((0.07 + 0.2 * unit(hash(band ^ 0x14u))) * (0.65 + 0.35 * strength)
    + accent * 0.05);
  let split = (0.012 + 0.03 * unit(hash(band ^ 0x15u))) * (0.6 + 0.4 * strength);
  var y = uv.y;
  if stutter {
    // The rows held: the picture of every fourth row drawn down the next ones.
    let hold = rows * 0.25;
    y = floor(y * hold) / hold;
  }
  let x = uv.x - off;
  var rgb = vec3f(
    load_uv(tex, vec2f(fract(x + split), y)).r,
    load_uv(tex, vec2f(fract(x), y)).g,
    load_uv(tex, vec2f(fract(x - split), y)).b,
  );
  let kind = unit(hash(band ^ 0x16u));
  if kind < 0.2 {
    rgb = vec3f(1.0) - rgb;
  } else if kind < 0.35 {
    rgb = hue_of(theme) * (0.12 + 1.1 * luma(rgb));
  }
  return min(rgb, vec3f(0.92));
}

// The mosaic, made to hit (the classic one was too gentle): big flat blocks of the picture,
// posterized to five levels with a touch of contrast, a whole region or the whole frame at once. As
// a lens effect a hit takes a region to blocks in steps with the beat, coarse to finer, and the
// picture comes back; as a switch the whole frame steps up to the coarsest cells, the new picture
// is behind them, and they step down again.

// The cell of a mosaic at its coarsest, as a share of the frame's height, at the least and at
// the most of a lens effect's amount.
const MOSAIC_CELL_LEAST: f32 = 0.05;
const MOSAIC_CELL_MOST: f32 = 0.12;
// The steps of a lens mosaic's hit, one per beat: the share of the coarsest cell; then it rests
// for `MOSAIC_REST` beats.
const MOSAIC_STEPS: vec3f = vec3f(1.0, 0.62, 0.38);
const MOSAIC_REST: u32 = 1u;

// The colour of a mosaic cell of `cell` pixels whose corner is at `origin` (pixels): the average
// of the picture at nine points of it.
fn cell_colour(tex: texture_2d<f32>, origin: vec2f, cell: f32) -> vec3f {
  var sum = vec3f(0.0);
  for (var j = 0u; j < 3u; j += 1u) {
    for (var i = 0u; i < 3u; i += 1u) {
      sum += load(tex, vec2i(origin + cell * (vec2f(f32(i), f32(j)) + 0.5) / 3.0)).rgb;
    }
  }
  return sum / 9.0;
}

// A block's colour: flat, in five levels, a touch more contrast, never full white.
fn posterize(c: vec3f) -> vec3f {
  let hard = clamp((c - 0.5) * 1.12 + 0.5, vec3f(0.0), vec3f(1.0));
  return min(floor(hard * 4.0 + 0.5) / 4.0, vec3f(0.92));
}

// The picture at `at` in cells of `cell` pixels (the grid from the frame's corner), flat and
// posterized.
fn pixelated(tex: texture_2d<f32>, at: vec2i, cell: f32) -> vec3f {
  let origin = floor((vec2f(at) + 0.5) / cell) * cell;
  return posterize(cell_colour(tex, origin, cell));
}

// A region of the frame for the hit of `id`: the whole frame, or a rectangle of it (x0, y0, x1,
// y1 as shares of the frame), from 45% by 40% up.
fn mosaic_region(id: u32) -> vec4f {
  if unit(hash(id ^ 0xe1u)) < 0.4 {
    return vec4f(0.0, 0.0, 1.0, 1.0);
  }
  let w = 0.45 + 0.4 * unit(hash(id ^ 0xe2u));
  let h = 0.4 + 0.4 * unit(hash(id ^ 0xe3u));
  let x = (1.0 - w) * unit(hash(id ^ 0xe4u));
  let y = (1.0 - h) * unit(hash(id ^ 0xe5u));
  return vec4f(x, y, x + w, y + h);
}

// The mosaic as a lens effect over the picture `tex` at `at`, on the eased beat time `t` (the
// whole beats are the steps): a hit lasts three beats in blocks of the coarsest size (a share of
// the height from 5% to 12% by `amount`, 0 to 1), then 62% of it, then 38%, and then the picture
// is back for one beat. Whether a hit comes is a roll against the amount (30% and up), so it rises and falls
// with the music. The region is picked by the hit.
fn mosaic_blocks(
  tex: texture_2d<f32>,
  at: vec2i,
  seed: u32,
  amount: f32,
  t: f32,
  accent: f32,
) -> vec3f {
  let here = load(tex, at).rgb;
  let beat = u32(max(floor(t), 0.0));
  let length = 3u + MOSAIC_REST;
  let hit = beat / length;
  let step = beat % length;
  if step >= 3u {
    return here;
  }
  let strength = clamp(amount, 0.0, 1.0);
  if unit(hash3(seed, hit, 0xd1u)) >= min(0.3 + 1.2 * strength, 1.0) {
    return here;
  }
  let size = vec2f(textureDimensions(tex));
  let uv = (vec2f(at) + 0.5) / size;
  let region = mosaic_region(hash3(seed, hit, 0xd2u));
  if any(uv < region.xy) || any(uv >= region.zw) {
    return here;
  }
  let coarsest = mix(MOSAIC_CELL_LEAST, MOSAIC_CELL_MOST, strength);
  let cell = max(round(size.y * coarsest * MOSAIC_STEPS[step]), 6.0);
  return pixelated(tex, at, cell);
}

// A mosaic switch at `progress`, in ten steps: the old picture in cells that step up from 3% of
// the height to 12% (the first two in a region of the frame, then the whole of it); at half way
// the new picture is behind the largest cells, which step down again to 2%.
fn mosaic_switch(
  before: texture_2d<f32>,
  after: texture_2d<f32>,
  at: vec2i,
  seed: u32,
  progress: f32,
) -> vec3f {
  let size = vec2f(textureDimensions(after));
  let uv = (vec2f(at) + 0.5) / size;
  let step = min(u32(progress * 10.0), 9u);
  var share = array<f32, 10>(0.03, 0.05, 0.08, 0.1, 0.12, 0.12, 0.09, 0.06, 0.035, 0.02);
  let cell = max(round(size.y * share[step]), 4.0);
  if step >= 5u {
    return pixelated(after, at, cell);
  }
  if step < 2u {
    let region = mosaic_region(seed);
    if any(uv < region.xy) || any(uv >= region.zw) {
      return load(before, at).rgb;
    }
  }
  return pixelated(before, at, cell);
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
      let e = transition.extra;
      if e.w > 0.5 {
        // Over the live picture, as a lens effect, moving continuously.
        let rgb = glitch_flow(fresh, at, transition.seed, e.x, e.y, transition.progress, transition.color.rgb);
        return vec4f(rgb, frame_now.a);
      }
      let subtle = e.z > 0.5;
      let rgb = glitch(held, fresh, at, transition.seed, e.x, e.y, subtle, transition.color.rgb);
      return vec4f(rgb, frame_now.a);
    }
    case KIND_MOSAIC: {
      let e = transition.extra;
      if e.x > 0.5 {
        // Over the live picture, as a lens effect.
        let rgb = mosaic_blocks(fresh, at, transition.seed, e.y, e.z, transition.progress);
        return vec4f(rgb, frame_now.a);
      }
      let rgb = mosaic_switch(held, fresh, at, transition.seed, transition.progress);
      return vec4f(rgb, frame_now.a);
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
