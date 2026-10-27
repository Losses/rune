// The cut-in of `transition.wgsl` (KIND_CUTIN), after Persona 5's: a ragged white stroke slashes
// across the old scene, a torn band bursts open out of it, overshoots and snaps back, and shows a
// close crop of the cover of the song playing (or, with none, of the scene in the theme colour);
// a beat later it rips along its length into strips that slide apart and thin out while splinters
// fly, and the new scene is behind. `fresh` is the old scene's live frame until the tear and the
// new scene's from then on (`transition.extra.z`); nothing is held. Everything is drawn in the
// theme colour (`transition.color`), black and an off-white, and the flash at the landing is
// capped.
//
// It moves in steps of whole frames at 60 fps, as a hand-drawn cut-in does, not in smooth fades.
// The geometry is in screen heights with the origin at the centre and y down. The band's own
// axes: `u` along it (rising to the right) and `s` across it. It is a wedge, pointed at the left
// and widest at the right, and its torn edges are a black line inside a white rim of big teeth
// and long splinters that lean back against the slash.
//
// Timing, in seconds since the start (`transition.cutin`): the slash and the opening take `band`
// (four frames), the band holds for `hold`, then it tears for `tear` (six frames).

const TAU: f32 = 6.2831853;
const CI_FPS: f32 = 60.0;
// The band rises by 15 degrees from left to right.
const CI_D = vec2f(0.96593, -0.25882);
const CI_N = vec2f(0.25882, 0.96593);
// Half the band's thickness where it is widest: over a third of the height in all.
const CI_HALF: f32 = 0.19;
const CI_PAPER = vec3f(0.93);
const CI_INK = vec3f(0.02, 0.02, 0.03);
// The strips the band rips into, across it.
const CI_STRIPS: i32 = 4;
const CI_SPLINTERS: i32 = 26;

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
  // How far inside the picture's edge the point is (negative outside it).
  inside: f32,
}

fn pcg(v: u32) -> u32 {
  let state = v * 747796405u + 2891336453u;
  let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
  return (word >> 22u) ^ word;
}

// 0 to 1, the same for the same `i` and `salt`, unrelated for another salt.
fn rnd(i: i32, salt: u32) -> f32 {
  return f32(pcg(u32(i) * 0x9e3779b1u + salt * 0x85ebca6bu) & 0xffffffu) / 16777216.0;
}

// Torn teeth along `x`: in cells of `cell`, a share `fill` of them has a tooth of a random place,
// length and height (0 to 1). A tooth is highest at its left end and slopes down to the right,
// so that it points back along the band.
fn ci_teeth(x: f32, cell: f32, salt: u32, fill: f32) -> f32 {
  let c = x / cell;
  let i = i32(floor(c));
  let f = c - floor(c);
  if rnd(i, salt) > fill {
    return 0.0;
  }
  let start = 0.05 + 0.35 * rnd(i, salt + 1u);
  let along = (f - start) / (0.3 + 0.6 * rnd(i, salt + 2u));
  if along < 0.0 || along > 1.0 {
    return 0.0;
  }
  return (0.35 + 0.65 * rnd(i, salt + 3u)) * (1.0 - along);
}

// Whole frames at 60 fps in `seconds`; a frame drawn a little early still counts.
fn ci_frames(seconds: f32) -> i32 {
  return i32(floor(seconds * CI_FPS + 0.25));
}

// How far along `u` the band reaches out of the screen on either side.
fn ci_ext(aspect: f32) -> f32 {
  return 0.5 * aspect / CI_D.x + 0.2;
}

// The band's thickness along it, 0 to 1: a wedge, pointed off the left edge, full from right of
// the centre.
fn ci_wedge(u: f32, aspect: f32) -> f32 {
  return pow(smoothstep(-ci_ext(aspect) - 0.1, 0.35, u), 0.6);
}

// Which part of the band the point `q` is in, for a band `scale` times its thickness that reaches
// up to `reach` along it, where it ends in a point. `loose` adds the scraps of paper loose near
// its edges.
fn ci_band(q: vec2f, scale: f32, reach: f32, aspect: f32, loose: bool) -> Band {
  let u = dot(q, CI_D);
  let s = dot(q, CI_N);
  if u > reach {
    return Band(CI_NONE, u, s, -1.0);
  }
  let tip = clamp((reach - u) / 0.3, 0.0, 1.0);
  let wedge = ci_wedge(u, aspect);
  let hh = CI_HALF * scale * wedge * tip;
  // The teeth keep some size where the band is thin, so that a thin stroke is ragged too.
  let amp = clamp(scale, 0.35, 1.4) * (0.35 + 0.65 * wedge) * tip;
  let salt = select(0u, 40u, s < 0.0);
  let a = abs(s);
  // The picture's edge wavers a little; the black line along it is torn into teeth of its own.
  let picture = hh * (1.0 - 0.06 * ci_teeth(u, 0.3, salt + 1u, 0.6));
  if a < picture {
    return Band(CI_BODY, u, s, picture - a);
  }
  let inked = a - picture;
  let black = amp * (0.007 + 0.02 * ci_teeth(u + 2.5 * inked, 0.07, salt + 2u, 0.5));
  if inked < black {
    return Band(CI_BLACK_PART, u, s, picture - a);
  }
  // The white rim: big teeth, small ones and long splinters, all leaning back as they reach
  // out, few of them in a run, so that the edge is torn and not serrated.
  let d = inked - black;
  let x = u + 2.4 * d;
  let rim = amp * (0.005 + 0.13 * ci_teeth(x, 0.24, salt + 3u, 0.55)
    + 0.03 * ci_teeth(x, 0.06, salt + 4u, 0.45));
  let splinter = amp * 0.085 * ci_teeth(u + 7.0 * d, 0.55, salt + 5u, 0.45);
  if d < max(rim, splinter) {
    return Band(CI_RIM_PART, u, s, picture - a);
  }
  if loose && d < 0.11 * amp {
    // A scrap in some cells along the edge: a thin tilted sliver, white or black.
    let cell = 0.07;
    let i = i32(floor(u / cell));
    if rnd(i, salt + 6u) < 0.5 {
      let du = u - (f32(i) + 0.25 + 0.5 * rnd(i, salt + 7u)) * cell;
      let half = 0.008 + 0.022 * rnd(i, salt + 8u);
      let thick = (0.002 + 0.004 * rnd(i, salt + 9u)) * amp;
      let dd = d - amp * (0.025 + 0.06 * rnd(i, salt + 10u)) - du * (rnd(i, salt + 11u) - 0.5);
      if abs(du) < half && abs(dd) < thick * (1.0 - abs(du) / half) {
        let part = select(CI_RIM_PART, CI_BLACK_PART, rnd(i, salt + 12u) < 0.3);
        return Band(part, u, s, picture - a);
      }
    }
  }
  return Band(CI_NONE, u, s, picture - a);
}

// The scene's frame at a pixel position, smoothly.
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

// What the band shows at `b`: a close crop of the cover, which pans and closes in slowly while
// the band holds, or without one the scene zoomed in, in black, the theme colour and paper. A
// glint (style 0) runs across it late in the hold.
fn ci_body(b: Band, aspect: f32, theme: vec3f, style: u32) -> vec3f {
  let t = transition.cutin.x;
  let band = transition.cutin.y;
  let hold = transition.cutin.z;
  let drift = clamp((t - band) / max(hold, 0.1), 0.0, 1.2);
  var c: vec3f;
  if transition.extra.y > 0.5 {
    let side = aspect * 1.1;
    let zoom = 1.0 - 0.06 * drift;
    let at = vec2f(b.u, b.s) / side * zoom + 0.5 + vec2f(0.05, 0.02) * (drift - 0.5);
    c = textureSampleLevel(cover, cover_sampler, at, 0.0).rgb;
  } else {
    let res = vec2f(textureDimensions(fresh));
    let q = CI_D * b.u + CI_N * b.s;
    let px = res * 0.5 + q * res.y / (1.7 + 0.1 * drift) + vec2f((drift - 0.5) * 0.04 * res.y, 0.0);
    // Lifted so that a dark scene still fills the band with the theme colour.
    let l = clamp(sqrt(luma(ci_fresh_at(px))) * 1.3, 0.0, 1.0);
    c = select(mix(theme, CI_PAPER, (l - 0.5) * 2.0), mix(theme * 0.35, theme, l * 2.0), l < 0.5);
  }
  if style != CI_HALFTONE {
    c = c * mix(0.75, 1.0, smoothstep(0.0, 0.04, b.inside));
  }
  if style == CI_STREAKS {
    // The glint is a bright diagonal sweep, capped short of white.
    let g = (t - (band + 0.6 * hold)) / max(0.25 * hold, 0.05);
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

fn ci_color(b: Band, aspect: f32, theme: vec3f, style: u32) -> vec3f {
  if b.part == CI_RIM_PART {
    return CI_PAPER;
  }
  if b.part == CI_BLACK_PART {
    return CI_INK;
  }
  return ci_body(b, aspect, theme, style);
}

// The halftone style's band: no rim and no black line, but dots on a grid turned by 45 degrees
// that grow from nothing outside the torn edge to a solid band inside it.
fn ci_halftone(b: Band, q: vec2f) -> bool {
  let coverage = smoothstep(-0.07, 0.05, b.inside);
  let g = vec2f(q.x + q.y, q.x - q.y) * (0.7071 / 0.022);
  return length(fract(g) - 0.5) < coverage * 0.75;
}

// How far the old picture is shaken, `k` frames after the band burst open.
fn ci_shake(k: i32) -> vec2f {
  switch k {
    case 0: { return vec2f(0.018, -0.011); }
    case 1: { return vec2f(-0.013, 0.008); }
    case 2: { return vec2f(0.007, -0.005); }
    case 3: { return vec2f(-0.003, 0.002); }
    default: { return vec2f(0.0); }
  }
}

// The band's thickness in the frames after it burst open: it overshoots and snaps back.
fn ci_burst(k: i32) -> f32 {
  switch k {
    case 0: { return 1.25; }
    case 1: { return 1.12; }
    case 2: { return 0.94; }
    case 3: { return 1.03; }
    default: { return 1.0; }
  }
}

// The band before it tears: the slash in frames 0 and 1, the band bursting open in white in frame
// 2, then the band, over the old scene playing on.
fn ci_before_tear(position: vec2f, q: vec2f, aspect: f32, theme: vec3f, style: u32) -> vec3f {
  let t = transition.cutin.x;
  let band = transition.cutin.y;
  let hold = transition.cutin.z;
  let frame = ci_frames(t);
  let ext = ci_ext(aspect);

  // The old scene, shaken as the band lands and darkened under it.
  let shake = ci_shake(frame - 2);
  let res = vec2f(textureDimensions(fresh));
  let at = clamp(vec2i(position + shake * res.y), vec2i(0), vec2i(res) - vec2i(1));
  var dark = 0.55;
  if frame <= 0 {
    dark = 1.0;
  } else if frame == 1 {
    dark = 0.8;
  } else if frame == 2 {
    dark = 0.65;
  }
  var col = textureLoad(fresh, at, 0).rgb * dark;

  if frame <= 1 {
    // The slash: a thin ragged stroke from the left, its tip pointed, half way across in the
    // first frame and all the way in the second.
    let reach = select(ext + 0.4, mix(-ext, ext, 0.45), frame <= 0);
    let b = ci_band(q, select(0.13, 0.06, frame <= 0), reach, aspect, false);
    if b.part != CI_NONE {
      col = CI_PAPER;
    }
    return col;
  }

  let landed = frame >= 3;
  // Behind the band: speed lines burst out from its middle as it lands, and thin out in steps.
  if style == CI_STREAKS && landed {
    let life = 1.0 - floor(clamp((t - band) / max(hold, 0.1), 0.0, 1.0) * 6.0) / 6.0;
    let n = 84.0;
    let angle = atan2(q.y, q.x) / TAU + 0.5;
    let line = i32(floor(angle * n));
    let across = fract(angle * n) - 0.5;
    let width = 0.12 + 0.4 * rnd(line, 2u);
    let start = 0.22 + 0.4 * rnd(line, 3u);
    let r = length(q);
    let grow = f32(frame - 2) * 0.5;
    if rnd(line, 1u) > 1.0 - 0.45 * life && abs(across) < width * 0.5 && r > start && r < grow {
      col = select(CI_INK, theme, rnd(line, 4u) > 0.35);
    }
  }

  let scale = ci_burst(frame - 2);
  let qb = q - 0.5 * shake;
  // Behind the band: translucent copies trail it, like the after-image of a blade.
  if style == CI_ECHO && landed {
    let lag = min(f32(frame - 2) / 3.0, 1.0);
    for (var k = 1; k <= 3; k++) {
      let shift = vec2f(-0.85, 0.5) * (f32(k) * 0.05 * lag);
      let copy = ci_band(qb - shift, scale, 10.0, aspect, false);
      if copy.part != CI_NONE {
        let tint = select(theme, CI_PAPER, copy.part == CI_RIM_PART);
        col = mix(col, tint, 0.34 / f32(k));
      }
    }
  }

  let b = ci_band(qb, scale, 10.0, aspect, landed);
  if b.part != CI_NONE {
    if !landed {
      // Bursting open: all white, with its black line.
      col = select(CI_PAPER, CI_INK, b.part == CI_BLACK_PART);
    } else if style == CI_HALFTONE {
      if ci_halftone(b, q) {
        col = ci_body(b, aspect, theme, style);
      }
    } else {
      col = ci_color(b, aspect, theme, style);
    }
  }

  // One frame of flash as the band lands and a weaker one after it, in the theme colour lifted
  // towards white and capped well under it.
  let lifted = mix(theme, vec3f(1.0), 0.5);
  let flood = lifted * (0.82 / max(max(lifted.r, lifted.g), max(lifted.b, 0.05)));
  if frame == 3 {
    col = mix(col, flood, 0.42);
  } else if frame == 4 {
    col = mix(col, flood, 0.14);
  }
  return col;
}

// How far the strips have slid apart along the band, `k` frames into the tear, in heights.
fn ci_slide(k: i32) -> f32 {
  switch k {
    case 0: { return 0.03; }
    case 1: { return 0.09; }
    case 2: { return 0.19; }
    case 3: { return 0.34; }
    case 4: { return 0.52; }
    default: { return 0.75; }
  }
}

// Where the crack between strip `j - 1` and strip `j` runs across the band, at `u`, as a share of
// the band's half thickness (-1 to 1): jagged, at two scales.
fn ci_crack(j: i32, u: f32) -> f32 {
  let salt = 100u + u32(j) * 8u;
  let wander = mix(rnd(i32(floor(u / 0.4)), salt + 1u), rnd(i32(floor(u / 0.4)) + 1, salt + 1u),
    smoothstep(0.0, 1.0, fract(u / 0.4)));
  return -1.0 + 2.0 * f32(j) / f32(CI_STRIPS) + (rnd(j, salt) - 0.5) * 0.2
    + 0.3 * (wander - 0.5) + 0.12 * ci_teeth(u, 0.08, salt + 2u, 0.6);
}

// The tear, `k` frames in, over the new scene: the band rips along its length into strips that
// slide apart in turn, thin out and go pale, its right end crumples into black and white scraps,
// and splinters fly off and linger a frame or two.
fn ci_tear(position: vec2f, q: vec2f, aspect: f32, theme: vec3f, style: u32, k: i32) -> vec3f {
  let ext = ci_ext(aspect);
  let fresh_px = textureLoad(fresh, vec2i(position), 0).rgb;
  var col = fresh_px * select(select(1.0, 0.8, k == 1), 0.6, k == 0);
  let kf = f32(k);
  var pale = 0.0;
  if k >= 1 {
    pale = min(0.2 + 0.25 * (kf - 1.0), 0.9);
  }

  // Far across from the band no strip, rim or crumpled piece reaches: those pixels skip them.
  let near = abs(dot(q, CI_N)) < CI_HALF * ci_wedge(dot(q, CI_D), aspect) + 0.3 + 0.03 * kf;
  for (var j = 0; j < CI_STRIPS && near; j++) {
    let salt = 200u + u32(j) * 8u;
    let side = select(-1.0, 1.0, j % 2 == 0);
    let off = CI_D * side * (0.6 + 0.8 * rnd(j, salt)) * ci_slide(k)
      + CI_N * (f32(j) - 2.0) * 0.012 * kf;
    var qj = q - off;
    // The right end crumples: pieces of it jump across the band.
    let crumple = ext - 0.2 - 0.18 * kf;
    var crumpled = false;
    let along = dot(qj, CI_D);
    if along > crumple {
      let i = i32(floor(along / 0.06));
      qj = qj + CI_N * (rnd(i, salt + 1u) - 0.5) * 0.04 * (kf + 1.0);
      crumpled = true;
    }
    let u = dot(qj, CI_D);
    let s = dot(qj, CI_N);
    let half = CI_HALF * max(ci_wedge(u, aspect), 0.02);
    let sn = s / half;
    // The strip lies between its cracks (the outer ones reach past the rim) and thins towards
    // its middle.
    let lo = select(-1.4, ci_crack(j, u), j > 0);
    let hi = select(1.4, ci_crack(j + 1, u), j < CI_STRIPS - 1);
    let thin = clamp(1.0 - kf * (0.22 + 0.1 * rnd(j, salt + 2u)), 0.0, 1.0);
    let edge = (0.5 * (hi - lo) * thin - abs(sn - 0.5 * (lo + hi))) * half;
    if edge <= 0.0 {
      continue;
    }
    let b = ci_band(qj, 1.0, 10.0, aspect, false);
    if b.part == CI_NONE {
      continue;
    }
    if crumpled {
      // Crumpled into pieces of black, some of paper.
      col = select(CI_INK, CI_PAPER, rnd(i32(floor(u / 0.06)), salt + 3u) < 0.3);
    } else if b.part != CI_BODY {
      col = ci_color(b, aspect, theme, style);
    } else {
      // The torn edge along each crack: white fibres, then a thin black line.
      let fibre = 0.005 + 0.03 * ci_teeth(u + 2.4 * edge, 0.12, salt + 4u, 0.6);
      if edge < fibre {
        col = CI_PAPER;
      } else if edge < fibre + 0.004 {
        col = CI_INK;
      } else {
        col = mix(ci_body(b, aspect, theme, style), CI_PAPER, pale);
      }
    }
    break;
  }

  // Splinters: thin needles of black or paper thrown off the band's edges, each gone after its
  // own number of frames.
  for (var i = 0; i < CI_SPLINTERS; i++) {
    let salt = 300u;
    if k > 2 + i32(rnd(i, salt) * 4.0) {
      continue;
    }
    let u0 = mix(-0.6 * ext, ext, rnd(i, salt + 1u));
    let side = select(-1.0, 1.0, rnd(i, salt + 2u) < 0.5);
    let s0 = side * CI_HALF * ci_wedge(u0, aspect) * (0.7 + 0.5 * rnd(i, salt + 3u));
    let velocity = CI_D * (rnd(i, salt + 4u) - 0.3) * 0.6
      + CI_N * side * (0.15 + 0.4 * rnd(i, salt + 5u));
    let centre = CI_D * u0 + CI_N * s0 + velocity * ci_slide(k) * 1.2;
    let rel = q - centre;
    let len = 0.03 + 0.15 * rnd(i, salt + 7u);
    if dot(rel, rel) > len * len {
      continue;
    }
    let turn = (rnd(i, salt + 6u) - 0.5) * 1.2 + side * 0.15 * kf;
    let axis = vec2f(cos(turn) * CI_D.x - sin(turn) * CI_D.y, sin(turn) * CI_D.x + cos(turn) * CI_D.y);
    let thick = 0.003 + 0.011 * rnd(i, salt + 8u);
    let x = dot(rel, axis);
    let y = abs(rel.x * axis.y - rel.y * axis.x);
    if abs(x) < len && y < thick * (1.0 - abs(x) / len) {
      col = select(CI_PAPER, CI_INK, rnd(i, salt + 9u) < 0.6);
    }
  }
  return col;
}

fn cutin(position: vec2f) -> vec4f {
  let res = vec2f(textureDimensions(fresh));
  let aspect = res.x / res.y;
  let q = (position - 0.5 * res) / res.y;
  let theme = transition.color.rgb;
  let style = u32(transition.extra.x + 0.5);
  var col: vec3f;
  if transition.extra.z > 0.5 {
    let since = transition.cutin.x - transition.cutin.y - transition.cutin.z;
    let k = clamp(ci_frames(since), 0, 5);
    col = ci_tear(position, q, aspect, theme, style, k);
  } else {
    col = ci_before_tear(position, q, aspect, theme, style);
  }
  return vec4f(clamp(col, vec3f(0.0), vec3f(1.0)), 1.0);
}
