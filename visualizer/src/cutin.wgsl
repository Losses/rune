// The cut-in of `transition.wgsl` (KIND_CUTIN), after Persona 5's: a ragged white stroke slashes
// across the old scene, a torn band bursts open out of it, overshoots and settles, and shows a
// close crop of the cover of the song playing (or, with none, of the scene in the theme colour);
// a beat later it cracks along its length, holds still a moment, and its three strips fly apart
// in turn, with the new scene behind. `fresh` is the old scene's live frame until the tear and
// the new scene's from then on (`transition.extra.z`); nothing is held. Everything is drawn in
// the theme colour (`transition.color`), black and an off-white, and the flash at the landing is
// capped.
//
// It is drawn on twos and threes, like hand-drawn animation: each pose holds for two or three
// frames at 60 fps and the next one comes at once, never a smooth fade.
// The geometry is in screen heights with the origin at the centre and y down. The band's own
// axes: `u` along it (rising to the right) and `s` across it. It is a wedge from a point off the
// left edge, its two edges at different angles (the top one opens much faster), so that no two
// long lines run side by side. Its torn edges are a thin black line right inside a white rim of
// teeth and splinters that lean back against the slash.
//
// Timing, in seconds since the start (`transition.cutin`): the slash and the opening take `band`
// (ten frames), the band holds for `hold`, then it tears for `tear` (ten frames). Frame by frame:
//   0-1 the slash half way across, 2-3 all the way, 4 the band bursts open in white,
//   5-6 the picture overshot with the flash, 7-8 undershot, 9 on, settled;
//   tear 0-2 cracked and still, 3-4, 5-6 and 7-8 the strips flying apart, 9 nearly out of the
//   screen.

const TAU: f32 = 6.2831853;
const CI_FPS: f32 = 60.0;
// The band's axis rises by 15 degrees from left to right.
const CI_D = vec2f(0.96593, -0.25882);
const CI_N = vec2f(0.25882, 0.96593);
// How fast the top and the bottom edge leave the axis along it, from the band's point: the top
// at about 6 degrees, the bottom at about 1.
const CI_TOP: f32 = 0.11;
const CI_BOT: f32 = 0.02;
// The axis passes this far below the centre of the screen, so that the band, which opens mostly
// upwards, lies over the centre.
const CI_DROP: f32 = 0.035;
const CI_PAPER = vec3f(0.93);
const CI_INK = vec3f(0.02, 0.02, 0.03);
// The strips the band cracks into, across it, and the shards the tear throws.
const CI_STRIPS: i32 = 3;
const CI_SHARDS: i32 = 6;

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

// How far along the band from its point, a little off the left edge of the screen.
fn ci_spread(u: f32, aspect: f32) -> f32 {
  return max(u + 0.5 * aspect * CI_D.x + 0.05, 0.0);
}

// The middle and the half thickness of the band at `u` across it (the top edge is at
// `x - y`, the bottom one at `x + y`).
fn ci_across(u: f32, aspect: f32) -> vec2f {
  let g = ci_spread(u, aspect);
  return vec2f(0.5 * (CI_BOT - CI_TOP) * g, 0.5 * (CI_BOT + CI_TOP) * g);
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
  let g = ci_spread(u, aspect) * tip;
  let top = s < 0.0;
  let edge = select(CI_BOT, CI_TOP, top) * scale * g;
  // The teeth keep some size where the band is thin, so that a thin stroke is ragged too.
  let amp = clamp(scale, 0.35, 1.3) * (0.3 + 0.7 * clamp(g / 1.2, 0.0, 1.0));
  let salt = select(0u, 40u, top);
  let a = abs(s);
  // The picture's edge wavers a little; a thin black line hugs it, torn a little itself.
  let picture = edge * (1.0 - 0.06 * ci_teeth(u, 0.3, salt + 1u, 0.6));
  if a < picture {
    return Band(CI_BODY, u, s, picture - a);
  }
  let inked = a - picture;
  let black = amp * (0.005 + 0.004 * ci_teeth(u + 2.5 * inked, 0.07, salt + 2u, 0.5));
  if inked < black {
    return Band(CI_BLACK_PART, u, s, picture - a);
  }
  // The white rim right outside it: teeth of two sizes and long splinters, all leaning back as
  // they reach out, few of them in a run, so that the edge is torn and not serrated.
  let d = inked - black;
  let x = u + 2.4 * d;
  let rim = amp * (0.009 + 0.07 * ci_teeth(x, 0.22, salt + 3u, 0.55)
    + 0.02 * ci_teeth(x, 0.06, salt + 4u, 0.45));
  let splinter = amp * 0.06 * ci_teeth(u + 7.0 * d, 0.55, salt + 5u, 0.4);
  if d < max(rim, splinter) {
    return Band(CI_RIM_PART, u, s, picture - a);
  }
  if loose && d < 0.05 * amp {
    // A scrap in some cells along the edge, close to it: a thin tilted sliver, white or black.
    let cell = 0.07;
    let i = i32(floor(u / cell));
    if rnd(i, salt + 6u) < 0.5 {
      let du = u - (f32(i) + 0.25 + 0.5 * rnd(i, salt + 7u)) * cell;
      let half = 0.008 + 0.018 * rnd(i, salt + 8u);
      let thick = (0.002 + 0.003 * rnd(i, salt + 9u)) * amp;
      let dd = d - amp * (0.014 + 0.022 * rnd(i, salt + 10u)) - du * (rnd(i, salt + 11u) - 0.5);
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

// How far the old picture is shaken, frame `f` of the opening: a jolt as the band bursts open,
// then back and settling, each held for two frames.
fn ci_shake(f: i32) -> vec2f {
  switch f {
    case 4: { return vec2f(0.016, -0.010); }
    case 5, 6: { return vec2f(-0.011, 0.007); }
    case 7, 8: { return vec2f(0.005, -0.003); }
    default: { return vec2f(0.0); }
  }
}

// The band's thickness in frame `f` of the opening: burst open, overshot, undershot, settled.
fn ci_burst(f: i32) -> f32 {
  switch f {
    case 4: { return 1.3; }
    case 5, 6: { return 1.2; }
    case 7, 8: { return 0.92; }
    default: { return 1.0; }
  }
}

// The band before it tears, over the old scene playing on (see the timing at the top).
fn ci_before_tear(position: vec2f, q: vec2f, aspect: f32, theme: vec3f, style: u32) -> vec3f {
  let t = transition.cutin.x;
  let band = transition.cutin.y;
  let hold = transition.cutin.z;
  let frame = ci_frames(t);
  let ext = ci_ext(aspect);

  // The old scene, shaken as the band lands and darkened under it in steps.
  let shake = ci_shake(frame);
  let res = vec2f(textureDimensions(fresh));
  let at = clamp(vec2i(position + shake * res.y), vec2i(0), vec2i(res) - vec2i(1));
  var dark = 0.55;
  if frame <= 1 {
    dark = 1.0;
  } else if frame <= 3 {
    dark = 0.85;
  } else if frame == 4 {
    dark = 0.7;
  }
  var col = textureLoad(fresh, at, 0).rgb * dark;

  if frame <= 3 {
    // The slash: a ragged stroke from the left with a pointed tip, half way across for two
    // frames, then all the way for two.
    let half_way = frame <= 1;
    let reach = select(ext + 0.4, mix(-ext, ext, 0.5), half_way);
    let b = ci_band(q, select(0.5, 0.3, half_way), reach, aspect, false);
    if b.part != CI_NONE {
      col = CI_PAPER;
    }
    return col;
  }

  let landed = frame >= 5;
  // Behind the band: speed lines burst out from its middle as it lands, reaching further every
  // two frames, and thin out in steps. None run along the band, beside its edges.
  if style == CI_STREAKS && landed {
    let life = 1.0 - floor(clamp((t - band) / max(hold, 0.1), 0.0, 1.0) * 6.0) / 6.0;
    let n = 84.0;
    let angle = atan2(q.y, q.x) / TAU + 0.5;
    let line = i32(floor(angle * n));
    let across = fract(angle * n) - 0.5;
    let width = 0.12 + 0.4 * rnd(line, 2u);
    let start = 0.22 + 0.4 * rnd(line, 3u);
    let r = length(q);
    let grow = f32((frame - 5) / 2 + 1) * 0.55;
    let middle = (f32(line) + 0.5) / n * TAU - 0.5 * TAU;
    let along_band = abs(sin(middle - atan2(CI_D.y, CI_D.x))) < 0.3;
    if !along_band && rnd(line, 1u) > 1.0 - 0.45 * life && abs(across) < width * 0.5
      && r > start && r < grow {
      col = select(CI_INK, theme, rnd(line, 4u) > 0.35);
    }
  }

  let scale = ci_burst(frame);
  let qb = q - 0.5 * shake;
  // Behind the band: translucent copies trail it, like the after-image of a blade.
  if style == CI_ECHO && landed {
    let lag = select(1.0, 0.5, frame <= 6);
    for (var k = 1; k <= 3; k++) {
      let shift = vec2f(-0.85, 0.5) * (f32(k) * 0.04 * lag);
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

  // The flash as the band lands, held for two frames, and a faint one after, in the theme
  // colour lifted towards white and capped well under it.
  let lifted = mix(theme, vec3f(1.0), 0.5);
  let flood = lifted * (0.82 / max(max(lifted.r, lifted.g), max(lifted.b, 0.05)));
  if frame == 5 || frame == 6 {
    col = mix(col, flood, 0.4);
  } else if frame == 7 {
    col = mix(col, flood, 0.1);
  }
  return col;
}

// The pose of the tear in its frame `k`: 0 while it holds cracked, then 1 to 4, each held for
// two frames.
fn ci_pose(k: i32) -> i32 {
  return select(min((k - 3) / 2 + 1, 4), 0, k < 3);
}

// How far the strips have slid apart along the band in a pose of the tear, in heights.
fn ci_slide(pose: i32) -> f32 {
  switch pose {
    case 0: { return 0.0; }
    case 1: { return 0.15; }
    case 2: { return 0.6; }
    case 3: { return 1.5; }
    default: { return 2.6; }
  }
}

// How thick the strips still are in a pose of the tear, of what they were.
fn ci_thin(pose: i32) -> f32 {
  switch pose {
    case 0, 1: { return 1.0; }
    case 2: { return 0.85; }
    default: { return 0.7; }
  }
}

// Where the crack between strip `j - 1` and strip `j` runs across the band, at `u`, as a share of
// the band's half thickness from its middle (-1 to 1): jagged, and wandering, so that it runs
// along neither edge.
fn ci_crack(j: i32, u: f32) -> f32 {
  let salt = 100u + u32(j) * 8u;
  let wander = mix(rnd(i32(floor(u / 0.4)), salt + 1u), rnd(i32(floor(u / 0.4)) + 1, salt + 1u),
    smoothstep(0.0, 1.0, fract(u / 0.4)));
  return -1.0 + 2.0 * f32(j) / f32(CI_STRIPS) + (rnd(j, salt) - 0.5) * 0.2
    + 0.4 * (wander - 0.5) + 0.1 * ci_teeth(u, 0.08, salt + 2u, 0.6);
}

// The tear, `k` frames in, over the new scene: the band cracks along its length into strips,
// holds still for three frames with the cracks open a hair, then the strips fly apart in turn
// along it, on twos, thinning, and a few shards fly off the cracks.
fn ci_tear(position: vec2f, q: vec2f, aspect: f32, theme: vec3f, style: u32, k: i32) -> vec3f {
  let pose = ci_pose(k);
  let step = f32(pose);
  var col = textureLoad(fresh, vec2i(position), 0).rgb * select(select(1.0, 0.8, pose == 1), 0.55, pose == 0);

  // Far across from the band no strip reaches: those pixels skip them.
  let across = ci_across(dot(q, CI_D), aspect);
  let near = abs(dot(q, CI_N) - across.x) < across.y + 0.12 + 0.03 * step;
  for (var j = 0; j < CI_STRIPS && near; j++) {
    let salt = 200u + u32(j) * 8u;
    let side = select(-1.0, 1.0, j % 2 == 0);
    let off = CI_D * side * (0.8 + 0.4 * rnd(j, salt)) * ci_slide(pose)
      + CI_N * (f32(j) - 1.0) * 0.012 * step;
    let qj = q - off;
    let u = dot(qj, CI_D);
    let s = dot(qj, CI_N);
    let frame = ci_across(u, aspect);
    let half = max(frame.y, 0.002);
    let sn = (s - frame.x) / half;
    // The strip lies between its cracks, each open by a hair (the outer bounds lie past the
    // rim), and thins towards its middle.
    let gap = 0.007 / half;
    let lo = select(-1.8, ci_crack(j, u) + gap, j > 0);
    let hi = select(1.8, ci_crack(j + 1, u) - gap, j < CI_STRIPS - 1);
    let edge = (0.5 * (hi - lo) * ci_thin(pose) - abs(sn - 0.5 * (lo + hi))) * half;
    if edge <= 0.0 {
      continue;
    }
    // Torn off at its right end too, past the screen, so that a strip flying left leaves it.
    let b = ci_band(qj, 1.0, ci_ext(aspect) + 0.3, aspect, false);
    if b.part == CI_NONE {
      continue;
    }
    if b.part != CI_BODY {
      col = ci_color(b, aspect, theme, style);
    } else {
      // The torn edge along each crack: white fibres right at it, then a thin black line.
      let fibre = 0.004 + 0.016 * ci_teeth(u + 2.4 * edge, 0.12, salt + 4u, 0.5);
      if edge < fibre {
        col = CI_PAPER;
      } else if edge < fibre + 0.003 {
        col = CI_INK;
      } else {
        col = ci_body(b, aspect, theme, style);
      }
    }
    break;
  }

  // A few shards off the cracks once the strips fly, each gone after a few poses.
  if pose >= 1 {
    for (var i = 0; i < CI_SHARDS; i++) {
      let salt = 300u;
      if pose > 2 + i32(rnd(i, salt) * 2.0) {
        continue;
      }
      let j = 1 + i % 2;
      let u0 = mix(-0.3, 0.75, rnd(i, salt + 1u));
      let frame = ci_across(u0, aspect);
      let side = select(-1.0, 1.0, rnd(i, salt + 2u) < 0.5);
      let centre = CI_D * u0 + CI_N * (frame.x + frame.y * ci_crack(j, u0))
        + (CI_D * side * 0.1 + CI_N * (rnd(i, salt + 3u) - 0.5) * 0.06) * step;
      let rel = q - centre;
      let len = 0.04 + 0.05 * rnd(i, salt + 4u);
      if dot(rel, rel) > len * len {
        continue;
      }
      let turn = (rnd(i, salt + 5u) - 0.5) * 0.8;
      let axis = vec2f(cos(turn) * CI_D.x - sin(turn) * CI_D.y, sin(turn) * CI_D.x + cos(turn) * CI_D.y);
      let thick = 0.004 + 0.004 * rnd(i, salt + 6u);
      let x = dot(rel, axis);
      let y = abs(rel.x * axis.y - rel.y * axis.x);
      if abs(x) < len && y < thick * (1.0 - abs(x) / len) {
        col = select(CI_PAPER, CI_INK, i < 4);
      }
    }
  }
  return col;
}

fn cutin(position: vec2f) -> vec4f {
  let res = vec2f(textureDimensions(fresh));
  let aspect = res.x / res.y;
  let q = (position - 0.5 * res) / res.y - CI_N * CI_DROP;
  let theme = transition.color.rgb;
  let style = u32(transition.extra.x + 0.5);
  var col: vec3f;
  if transition.extra.z > 0.5 {
    let since = transition.cutin.x - transition.cutin.y - transition.cutin.z;
    let k = clamp(ci_frames(since), 0, 9);
    col = ci_tear(position, q, aspect, theme, style, k);
  } else {
    col = ci_before_tear(position, q, aspect, theme, style);
  }
  return vec4f(clamp(col, vec3f(0.0), vec3f(1.0)), 1.0);
}
