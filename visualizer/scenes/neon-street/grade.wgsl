// The grade before the runtime's tone mapping.
//
// With a palette the picture is mapped by its lightness onto a single colour, as the city in
// Persona 5's art is drawn: in reds (black, deep red, red, white), in the player's theme colour
// the same way, or in ink (black, greys, white) where only the red lights stay red. The default
// palette follows the look the runtime has chosen for the section (see `[[look]]`): the grey look,
// a darker night in ink where the theme colour barely shows (a faint tint, the brightest red
// lights in its tone), at quiet and ordinary sections; and the theme colour look, the picture in
// its hue, at the climax. Posterizing flattens the tones into a few steps, like a print. The light
// itself is kept, so what glows still glows.

fn ramp(t: f32, a: vec3f, b: vec3f, c: vec3f, d: vec3f) -> vec3f {
  let x = clamp(t, 0.0, 1.0) * 3.0;
  if (x < 1.0) {
    return mix(a, b, x);
  }
  if (x < 2.0) {
    return mix(b, c, x - 1.0);
  }
  return mix(c, d, x - 2.0);
}

// A colour with the lightness l.
fn at_lightness(c: vec3f, l: f32) -> vec3f {
  return c / max(rv_luma(c), 1e-4) * l;
}

// The kinds of grade a picture is mapped by.
const REDS: u32 = 0u;
const INK: u32 = 1u;
const THEME: u32 = 2u;
const GREY: u32 = 3u;

// How dark the grey look is against the colour one, and how much of the theme colour shows in it
// (a tint of the greys, and the share of the brightest red lights kept in the theme's tone).
const GREY_DIM: f32 = 0.6;
const GREY_TINT: f32 = 0.07;
const GREY_ACCENT: f32 = 0.3;

// The picture's colour `c` (lightness l) as grade `kind` draws it, from its tone t and the
// lightness lt the tone stands for.
fn mapped(kind: u32, c: vec3f, l: f32, t: f32, lt: f32) -> vec3f {
  var g = vec3f(1.0);
  var dim = 1.0;
  if (kind == THEME) {
    g = ramp(t, state.theme_ramp[0].rgb, state.theme_ramp[1].rgb, state.theme_ramp[2].rgb, state.theme_ramp[3].rgb);
  } else if (kind == REDS) {
    g = ramp(t, vec3f(0.03, 0.0, 0.006), vec3f(0.4, 0.02, 0.035), vec3f(1.0, 0.07, 0.06), vec3f(1.0, 0.88, 0.86));
  } else if (kind == GREY) {
    g = mix(vec3f(1.0), state.theme_ramp[2].rgb / max(rv_luma(state.theme_ramp[2].rgb), 1e-3), GREY_TINT);
    dim = GREY_DIM;
  }
  var out = at_lightness(g, lt * dim);
  // The darkest step solid black, as P5 draws its shadows: whatever is dark becomes a black
  // shape (the dots between it and the next step stay).
  if (t < 1.0 / max(params.posterize, 2.0)) {
    out *= 1.0 - params.crush;
  }
  // In ink only the brightest red lights (beacons, lanterns, neon tubes) keep a colour; red
  // boards turn grey like the rest. In ink that colour is red, in the grey look the theme's tone
  // and only a little of it.
  if (kind == INK || kind == GREY) {
    let themed = kind == GREY;
    let accent = select(vec3f(1.0, 0.05, 0.04), state.theme_ramp[2].rgb / max(rv_luma(state.theme_ramp[2].rgb), 1e-3) * 0.35, themed);
    let redness = clamp((c.r - max(c.g, c.b)) / max(c.r, 1e-4), 0.0, 1.0) * smoothstep(1.2, 2.5, c.r);
    out = mix(out, at_lightness(accent, max(l, lt * dim)), redness * params.accent * select(1.0, GREY_ACCENT, themed));
  }
  return out;
}

fn shade(px: Pixel) -> vec4f {
  var c = textureSampleLevel(trail, linear_clamp, px.uv, 0.0).rgb;
  let l = rv_luma(c);
  // 0 the scene's own colours, 1 red, 2 ink, 3 the theme colour, 4 by look (grey or the theme
  // colour, as the runtime's looks say).
  let palette = params.palette;
  if (palette == 0u) {
    let shadows = 1.0 - smoothstep(0.0, 0.15, l);
    c *= mix(vec3f(1.0), vec3f(0.85, 0.9, 1.2), shadows * 0.5 * params.cool_shadows);
    return vec4f(c, 1.0);
  }

  // The lightness as a tone from 0 to 1, in a few flat steps when posterized.
  var t = l / (l + params.palette_key);
  if (params.posterize >= 2.0) {
    let n = params.posterize;
    // Each step flat; only where one gives way to the next, a printed screen of dots (as comics
    // shade): the nearer the next step, the bigger the dots, on a grid turned 45 degrees.
    var rise = smoothstep(0.42, 0.58, fract(t * n));
    if (params.halftone > 0.0) {
      let cover = smoothstep(0.3, 0.7, fract(t * n));
      let g = mat2x2f(0.7071, -0.7071, 0.7071, 0.7071) * px.coord / params.halftone;
      let r = length(fract(g) - 0.5);
      rise = 1.0 - smoothstep(-0.7, 0.7, (r - 0.71 * sqrt(cover)) * params.halftone);
    }
    t = (floor(t * n) + rise) / n;
  }
  let lt = t * params.palette_key / max(1.0 - t, 0.02);
  var out: vec3f;
  if (palette == 1u) {
    out = mapped(REDS, c, l, t, lt);
  } else if (palette == 2u) {
    out = mapped(INK, c, l, t, lt);
  } else if (palette == 3u) {
    out = mapped(THEME, c, l, t, lt);
  } else if (state.colour < 0.001) {
    out = mapped(GREY, c, l, t, lt);
  } else if (state.colour > 0.999) {
    out = mapped(THEME, c, l, t, lt);
  } else {
    // Fading from one look to the other.
    out = mix(mapped(GREY, c, l, t, lt), mapped(THEME, c, l, t, lt), state.colour);
  }
  return vec4f(out, 1.0);
}
