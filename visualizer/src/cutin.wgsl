// The cut-in of `transition.wgsl` (KIND_CUTIN): a diagonal band slashes across the frozen,
// darkened old picture, shows the cover of the song playing (or, with none, a crop of the new
// scene), and tears into scraps that fly off to the upper right, revealing the new scene. `held`
// is the old picture and `fresh` the new scene's frame. Everything is drawn in the theme colour
// (`transition.color`), black and an off-white, and the flash at the landing is capped.
//
// The geometry is in screen heights with the origin at the centre and y down. The band's own
// axes: `u` along it (rising to the right) and `s` across it.
//
// Timing, in seconds since the start (`transition.cutin`): the slash and the opening take `band`
// (two frames), the band holds for `hold`, then it tears for `tear`.

const TAU: f32 = 6.2831853;
// The band rises by 12 degrees from left to right.
const CI_D = vec2f(0.97815, -0.20791);
const CI_N = vec2f(0.20791, 0.97815);
// Half the band's thickness: a third of the height in all.
const CI_HALF: f32 = 0.16667;
const CI_RIM: f32 = 0.012;
const CI_BLACK: f32 = 0.007;
const CI_PAPER = vec3f(0.94);
const CI_INK = vec3f(0.02, 0.02, 0.03);
// The scraps: columns along the band by rows across it.
const CI_COLS: i32 = 12;
const CI_ROWS: i32 = 3;

const CI_STREAKS: u32 = 0u;
const CI_HALFTONE: u32 = 1u;
const CI_ECHO: u32 = 2u;

// What a point is in the band.
const CI_NONE: u32 = 0u;
const CI_RIM_PART: u32 = 1u;
const CI_BLACK_PART: u32 = 2u;
const CI_BODY: u32 = 3u;

struct Band {
  part: u32,
  u: f32,
  s: f32,
  // How far inside the edge the point is (negative in the rim).
  inside: f32,
}

fn pcg(v: u32) -> u32 {
  let state = v * 747796405u + 2891336453u;
  let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
  return (word >> 22u) ^ word;
}

// 0 to 1, the same for the same `i`.
fn rnd(i: i32) -> f32 {
  return f32(pcg(u32(i)) & 0xffffffu) / 16777216.0;
}

fn vnoise(x: f32) -> f32 {
  let i = i32(floor(x));
  let f = fract(x);
  return mix(rnd(i), rnd(i + 1), f * f * (3.0 - 2.0 * f));
}

// The new scene's frame at a pixel position, smoothly.
fn ci_fresh_at(px: vec2f) -> vec3f {
  let dim = vec2i(textureDimensions(fresh)) - vec2i(1);
  let p = px - 0.5;
  let i = vec2i(floor(p));
  let f = p - vec2f(i);
  let a = textureLoad(fresh, clamp(i, vec2i(0), dim), 0).rgb;
  let b = textureLoad(fresh, clamp(i + vec2i(1, 0), vec2i(0), dim), 0).rgb;
  let c = textureLoad(fresh, clamp(i + vec2i(0, 1), vec2i(0), dim), 0).rgb;
  let d = textureLoad(fresh, clamp(i + vec2i(1, 1), vec2i(0), dim), 0).rgb;
  return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

// Which part of the band the point `q` is in. The edges are torn: they wander, and the white
// rim outside the black inner edge changes its width in fibres. `hh` is the half thickness
// and the band exists up to `reach` along it. Thin (the slash) the edges do not wander.
fn ci_band(q: vec2f, hh: f32, reach: f32, style: u32) -> Band {
  let u = dot(q, CI_D);
  let s = dot(q, CI_N);
  if u > reach {
    return Band(CI_NONE, u, s, 0.0);
  }
  let thin = clamp(hh / 0.05, 0.0, 1.0);
  let top = -hh + ((vnoise(u * 13.0 + 3.7) - 0.5) * 0.034 + (vnoise(u * 41.0) - 0.5) * 0.014) * thin;
  let bot = hh + ((vnoise(u * 11.0 + 9.1) - 0.5) * 0.034 + (vnoise(u * 37.0 + 5.0) - 0.5) * 0.014) * thin;
  let inside = min(s - top, bot - s);
  if style == CI_HALFTONE && thin >= 1.0 {
    // No rim and no black edge: dots that grow from nothing outside the edge to a solid band
    // inside it, on a grid turned by 45 degrees.
    let coverage = smoothstep(-0.03, 0.06, inside);
    let g = vec2f(q.x + q.y, q.x - q.y) * (0.7071 / 0.024);
    let r = length(fract(g) - 0.5);
    if r >= coverage * 0.75 {
      return Band(CI_NONE, u, s, inside);
    }
    return Band(CI_BODY, u, s, inside);
  }
  let top_edge = (s - top) < (bot - s);
  let salt = select(0, 17, top_edge);
  let rim = (CI_RIM * 0.5 + CI_RIM * vnoise(u * 53.0 + f32(salt))
    + 0.008 * rnd(i32(floor(u * 160.0)) + salt)) * max(thin, 0.3);
  if inside < -rim {
    return Band(CI_NONE, u, s, inside);
  }
  if inside < 0.0 {
    return Band(CI_RIM_PART, u, s, inside);
  }
  if inside < CI_BLACK * thin {
    return Band(CI_BLACK_PART, u, s, inside);
  }
  return Band(CI_BODY, u, s, inside);
}

// What the band shows: the cover, which pans slowly while the band holds, or the new scene
// zoomed in. A glint (style 0) runs across it late in the hold.
fn ci_body(b: Band, aspect: f32, theme: vec3f, style: u32) -> vec3f {
  let t = transition.cutin.x;
  let band = transition.cutin.y;
  let hold = transition.cutin.z;
  let drift = clamp((t - band) / max(hold, 0.1), 0.0, 1.5);
  var c: vec3f;
  if transition.extra.y > 0.5 {
    let side = aspect * 1.15;
    let at = vec2f(0.5 + b.u / side + (drift - 0.5) * 0.06, 0.5 + b.s / side + (drift - 0.5) * 0.2);
    c = textureSampleLevel(cover, cover_sampler, at, 0.0).rgb;
  } else {
    let res = vec2f(textureDimensions(fresh));
    let q = CI_D * b.u + CI_N * b.s;
    let px = res * 0.5 + q * res.y;
    let crop = res * 0.5 + (px - res * 0.5) / 1.7 + vec2f((drift - 0.5) * 0.08 * res.x, 0.0);
    c = ci_fresh_at(crop);
  }
  if style != CI_HALFTONE {
    c = c * mix(0.72, 1.0, smoothstep(0.0, 0.05, b.inside));
  }
  if style == CI_STREAKS {
    // The glint is a bright diagonal sweep, capped short of white.
    let g = (t - (band + 0.55 * hold)) / max(0.4 * hold, 0.05);
    if g >= 0.0 && g <= 1.0 {
      let ext = 0.5 * aspect + 0.25;
      let w = b.u + b.s * 0.9 - mix(-ext, ext, g);
      let profile = exp(-pow(w / 0.05, 2.0)) + 0.4 * exp(-pow((w - 0.11) / 0.02, 2.0));
      let glint = mix(theme, vec3f(1.0), 0.5) * profile * 0.55;
      c = min(c + glint, max(c, vec3f(0.88)));
    }
  }
  return c;
}

fn ci_color(b: Band, aspect: f32, theme: vec3f, style: u32, slash: f32) -> vec3f {
  var c = ci_body(b, aspect, theme, style);
  if b.part == CI_RIM_PART {
    c = CI_PAPER;
  } else if b.part == CI_BLACK_PART {
    c = CI_INK;
  }
  // The slash that opens the band is a white line.
  return mix(c, CI_PAPER, slash);
}

fn cutin(position: vec2f) -> vec4f {
  let res = vec2f(textureDimensions(fresh));
  let at = vec2i(position);
  let aspect = res.x / res.y;
  let q = (position - 0.5 * res) / res.y;
  let t = transition.cutin.x;
  let band = transition.cutin.y;
  let hold = transition.cutin.z;
  let tear = transition.cutin.w;
  let theme = transition.color.rgb;
  let style = u32(transition.extra.x + 0.5);

  let hold_end = band + hold;
  let p = clamp(t / band, 0.0, 1.0);
  let tearing = t >= hold_end;
  let tp = clamp((t - hold_end) / max(tear, 0.001), 0.0, 1.0);

  // The old picture, darkened as the band opens, until the tear shows the new scene.
  let old_px = textureLoad(held, at, 0).rgb;
  let new_px = textureLoad(fresh, at, 0).rgb;
  var col = old_px * mix(1.0, 0.3, smoothstep(0.0, 0.6, p));
  if tearing {
    col = mix(col, new_px, smoothstep(0.0, 0.25, tp));
  }

  // The slash runs in from the left and the band opens around it.
  let ext = 0.5 * aspect + 0.3;
  let hh = mix(0.004, CI_HALF, smoothstep(0.35, 1.0, p));
  let reach = mix(-ext, ext, clamp(0.25 + p * 1.4, 0.0, 1.0));
  let slash = 1.0 - smoothstep(0.3, 0.65, p);

  // Behind the band: speed lines burst out from its middle as it lands, then die away.
  if style == CI_STREAKS && !tearing {
    let landed = (t - band) / max(hold, 0.1);
    let life = 1.0 - smoothstep(0.15, 0.9, landed);
    let n = 84.0;
    let angle = atan2(q.y, q.x) / TAU + 0.5;
    let line = i32(floor(angle * n));
    let across = fract(angle * n) - 0.5;
    let width = 0.18 + 0.5 * rnd(line * 4 + 2);
    let start = 0.22 + 0.4 * rnd(line * 4 + 3);
    let r = length(q);
    if rnd(line * 4 + 1) > 0.4 && abs(across) < width * 0.5 && r > start && r < (t / band) * 1.4 {
      let c = select(CI_INK, theme, rnd(line * 4 + 4) > 0.35);
      col = mix(col, c, 0.9 * life);
    }
  }

  // Behind the band: translucent copies trail it, like the after-image of a blade.
  if style == CI_ECHO {
    let lag = 1.0 - exp(-max(t - band * 0.5, 0.0) * 25.0);
    for (var k = 1; k <= 3; k++) {
      let shift = vec2f(-0.85, 0.5) * (f32(k) * 0.05 * lag);
      let copy = ci_band(q - shift, hh, reach, CI_STREAKS);
      if copy.part != CI_NONE {
        let tint = select(theme, CI_PAPER, copy.part == CI_RIM_PART);
        col = mix(col, tint, 0.34 / f32(k) * (1.0 - tp));
      }
    }
  }

  if !tearing {
    let b = ci_band(q, hh, reach, style);
    if b.part != CI_NONE {
      col = ci_color(b, aspect, theme, style, slash);
    }
  } else {
    // The band tears into scraps: a grid of them along and across the band. Those at the right
    // go first, and each flies off to the upper right, turning, faster as it goes.
    let cw = 2.0 * ext / f32(CI_COLS);
    let rows = (2.0 * CI_HALF + 0.07) / f32(CI_ROWS);
    for (var k = 0; k < CI_COLS * CI_ROWS; k++) {
      let ci = k % CI_COLS;
      let ri = k / CI_COLS;
      let cu = -ext + (f32(ci) + 0.5) * cw;
      let cs = -CI_HALF - 0.035 + (f32(ri) + 0.5) * rows;
      let h = vec4f(rnd(k * 5 + 1), rnd(k * 5 + 2), rnd(k * 5 + 3), rnd(k * 5 + 4));
      let delay = 0.3 * (1.0 - f32(ci) / f32(CI_COLS - 1));
      let tk = clamp((tp - delay) / 0.7, 0.0, 1.0);
      let dir = normalize(vec2f(1.0, -0.8) + (h.xy - 0.5) * 0.7);
      let fly = dir * (0.7 + 1.3 * h.z) * tk * tk * 1.6;
      let angle = (h.w - 0.5) * 3.0 * tk;
      let centre = CI_D * cu + CI_N * cs;
      let rel = q - centre - fly;
      let loc = vec2f(
        cos(angle) * rel.x + sin(angle) * rel.y,
        -sin(angle) * rel.x + cos(angle) * rel.y,
      );
      let src = centre + loc;
      let du = dot(loc, CI_D);
      let ds = dot(loc, CI_N);
      // A rectangle with a torn corner.
      let phi = h.x * TAU;
      let corner = 0.045 + 0.05 * h.y - dot(vec2f(du, ds), vec2f(cos(phi), sin(phi)));
      let inside = min(min(cw * 0.5 - abs(du), rows * 0.5 - abs(ds)), corner);
      if inside > 0.0 {
        let b = ci_band(src, CI_HALF, 10.0, style);
        if b.part != CI_NONE {
          var c = ci_color(b, aspect, theme, style, 0.0);
          if inside < 0.006 {
            c = CI_PAPER;
          }
          col = mix(col, c, 1.0 - smoothstep(0.75, 1.0, tp));
        }
      }
    }
  }

  // The flash as the band lands, in the theme colour brought to a capped brightness.
  let since = t - 0.6 * band;
  if since >= 0.0 {
    let flash = theme * (0.85 / max(max(theme.r, theme.g), max(theme.b, 0.05)));
    col = mix(col, min(flash, vec3f(0.85)), 0.55 * exp(-since / 0.06));
  }
  return vec4f(clamp(col, vec3f(0.0), vec3f(1.0)), 1.0);
}
