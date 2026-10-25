// The grade before the runtime's tone mapping.
//
// With a palette the picture is mapped by its lightness onto a single colour, as the city in
// Persona 5's art is drawn: in reds (black, deep red, red, white), in the player's theme colour
// the same way, or in ink (black, greys, white) where only the red lights stay red. By section
// reds or the theme colour take turns with ink, each kind of section keeping its own. Posterizing flattens the tones into a few steps, like a print. The light
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

fn shade(px: Pixel) -> vec4f {
  var c = textureSampleLevel(trail, linear_clamp, px.uv, 0.0).rgb;
  let l = rv_luma(c);
  // 0 the scene's own colours, 1 red, 2 ink, 3 red or ink by section, 4 the theme colour, 5 the
  // theme colour or ink by section.
  var mode = f32(params.palette);
  let ink_turn = fract(state.district * 2.0) >= 0.5;
  // The accent that stays coloured among the ink and colours the wipe at a cut: red, or the
  // theme colour when the palette is the theme's.
  let themed = mode > 3.5;
  let accent = select(vec3f(1.0, 0.05, 0.04), state.theme_ramp[2].rgb / max(rv_luma(state.theme_ramp[2].rgb), 1e-3) * 0.35, themed);
  if (mode > 4.5) {
    mode = select(4.0, 2.0, ink_turn);
  } else if (mode > 2.5 && mode < 3.5) {
    mode = select(1.0, 2.0, ink_turn);
  }
  if (mode < 0.5) {
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
  var g: vec3f;
  if (mode > 3.5) {
    g = ramp(t, state.theme_ramp[0].rgb, state.theme_ramp[1].rgb, state.theme_ramp[2].rgb, state.theme_ramp[3].rgb);
  } else if (mode < 1.5) {
    g = ramp(t, vec3f(0.03, 0.0, 0.006), vec3f(0.4, 0.02, 0.035), vec3f(1.0, 0.07, 0.06), vec3f(1.0, 0.88, 0.86));
  } else {
    g = vec3f(1.0);
  }
  var out = at_lightness(g, lt);
  // The darkest step solid black, as P5 draws its shadows: whatever is dark becomes a black
  // shape (the dots between it and the next step stay).
  if (t < 1.0 / max(params.posterize, 2.0)) {
    out *= 1.0 - params.crush;
  }
  // In ink only the brightest red lights (beacons, lanterns, neon tubes) keep a colour, the
  // accent's; red boards turn grey like the rest.
  if (mode > 1.5 && mode < 2.5) {
    let redness = clamp((c.r - max(c.g, c.b)) / max(c.r, 1e-4), 0.0, 1.0) * smoothstep(1.2, 2.5, c.r);
    out = mix(out, at_lightness(accent, max(l, lt)), redness * params.accent);
  }
  return vec4f(cut_wipe(px, out, accent), 1.0);
}

// At a cut, a band of black and the accent sweeping across the picture on a slant, its edges jagged
// and its tail breaking into dots (as Persona 5 turns from one scene to the next), over
// `wipe_time` seconds; the new view is already there behind it.
fn cut_wipe(px: Pixel, c: vec3f, accent: vec3f) -> vec3f {
  let age = frame.time - state.last_cut;
  if (params.wipe_time <= 0.0 || age < 0.0 || age > params.wipe_time || frame.time < 0.5) {
    return c;
  }
  let p = age / params.wipe_time;
  let dir = normalize(vec2f(1.0, -0.45));
  let side = vec2f(-dir.y, dir.x);
  let aspect = px.size.x / px.size.y;
  let reach = 0.5 * (aspect + 1.0);
  // Across the band, from its front edge back (in screen heights); jagged by a sawtooth along it.
  let along = dot(px.p, side);
  let jag = (h21(vec2f(floor(along * 9.0), 3.7)) - 0.5) * 0.08 + abs(fract(along * 9.0) - 0.5) * 0.06;
  let front = mix(-reach - 0.2, reach + 1.4, p);
  let behind = front - (dot(px.p, dir) + jag);
  if (behind < 0.0 || behind > 1.2) {
    return c;
  }
  // The accent at the front, then black, then dots thinning out into the new view.
  let red = at_lightness(accent, 0.22);
  if (behind < 0.18) {
    return red;
  }
  if (behind < 0.7) {
    return vec3f(0.0);
  }
  let g = mat2x2f(0.7071, -0.7071, 0.7071, 0.7071) * px.coord / 7.0;
  let dot_on = length(fract(g) - 0.5) < 0.62 * (1.0 - (behind - 0.7) / 0.5);
  return select(c, vec3f(0.0), dot_on);
}
