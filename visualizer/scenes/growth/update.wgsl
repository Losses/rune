// Once per frame: the tower shoots up on every beat (more on a bar's first), wide and full where
// the music is loud; the camera rising after it and circling; the rod breathing with the energy;
// each section seen its own way. A plate once grown never changes.

// ---- Colour ----
// The themes are hand made, each with its own hues. Following the player's colour (its accent,
// which follows the system, the listener's choice or the cover), every colour of the theme keeps
// how light and how colourful it is (held down, so it is never loud) and takes the player's hue:
// the lights turned a few degrees warmer, the sky and haze a few cooler, so they still differ a
// little, the plates just the hue. All in OKLab, where a step in lightness or hue looks as big as
// it is. The greys of concrete stay grey.

fn oklab(c: vec3f) -> vec3f {
  let l = pow(max(0.4122214708 * c.r + 0.5363325363 * c.g + 0.0514459929 * c.b, 0.0), 1.0 / 3.0);
  let m = pow(max(0.2119034982 * c.r + 0.6806995451 * c.g + 0.1073969566 * c.b, 0.0), 1.0 / 3.0);
  let s = pow(max(0.0883024619 * c.r + 0.2817188376 * c.g + 0.6299787005 * c.b, 0.0), 1.0 / 3.0);
  return vec3f(0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s, 1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s, 0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s);
}

// A colour (linear) from OKLCH: lightness, chroma, hue in radians.
fn oklch(lightness: f32, chroma: f32, hue: f32) -> vec3f {
  let a = chroma * cos(hue);
  let b = chroma * sin(hue);
  let l = pow(lightness + 0.3963377774 * a + 0.2158037573 * b, 3.0);
  let m = pow(lightness - 0.1055613458 * a - 0.0638541728 * b, 3.0);
  let s = pow(lightness - 0.0894841775 * a - 1.2914855480 * b, 3.0);
  return max(vec3f(4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s, -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s, -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s), vec3f(0.0));
}

// Colour c in hue `hue` (with its chroma held to `most`).
fn rehue(c: vec3f, hue: f32, most: f32) -> vec3f {
  let lab = oklab(c);
  return oklch(lab.x, min(length(lab.yz), most), hue);
}

// The looks on screen: the one before, the one now and how far the fade is, all the runtime's
// (`look`, see scene.toml's [[look]]s). The `light` parameter pins one and overrides it.
fn looks_now() -> vec3f {
  if (params.light > 0u) {
    let pinned = f32(params.light - 1u);
    return vec3f(pinned, pinned, 1.0);
  }
  return vec3f(f32(look.prev), f32(look.to), look.mix);
}

// The colours now: the last look fading into the next; in the player's colour if it follows it
// and the player has told it.
fn set_theme() {
  let now = looks_now();
  let a = theme_of(now.x);
  let b = theme_of(now.y);
  // The runtime's mix is eased already.
  let k = now.z;
  var th = Theme(mix(a.haze_up, b.haze_up, k), mix(a.haze_down, b.haze_down, k), mix(a.glow, b.glow, k),
    mix(a.pale, b.pale, k), mix(a.mid, b.mid, k), mix(a.dark, b.dark, k), mix(a.odd, b.odd, k),
    mix(a.key, b.key, k), mix(a.sky_up, b.sky_up, k), mix(a.sky_down, b.sky_down, k),
    mix(a.rod, b.rod, k), mix(a.dust, b.dust, k), normalize(mix(a.sun, b.sun, k)));
  if (params.player_colour == 1u && frame.accent.a > 0.5) {
    let lab = oklab(frame.accent.rgb);
    let h = atan2(lab.z, lab.y);
    let most = 0.12 * params.chroma;
    // Warmer is towards amber (about 1 radian) the short way round.
    let warm = h + 0.15 * sign(atan2(sin(1.0 - h), cos(1.0 - h)));
    let cool = 2.0 * h - warm;
    th = Theme(rehue(th.haze_up, cool, most), rehue(th.haze_down, cool, most), rehue(th.glow, warm, most),
      rehue(th.pale, h, most), rehue(th.mid, h, most), rehue(th.dark, h, most), rehue(th.odd, warm, most),
      rehue(th.key, warm, most), rehue(th.sky_up, cool, most), rehue(th.sky_down, cool, most),
      rehue(th.rod, warm, most), rehue(th.dust, h, most), th.sun);
  }
  state.theme = th;
}

fn random(n: u32) -> f32 {
  return rv_unit(frame.seed * 2654435761u + n);
}

// Take the view the director cut to, at once: a cut is a cut, never a slide. The views, in the
// order of scene.toml's shots: 0 steep, 1 near and 2 far look up the tower from below (so what
// is under the camera never needs drawing); 3 looks down on the tip from not far above. Then
// the burst's four, one a beat before a section that lifts: under the tip, close on its other
// side, a plate landing at the growing front through a longer lens, and high above the tip, the
// tower's rings growing up at the lens. The camera circles the same way the whole song (turning
// back would brake and speed up again), each of the burst's cuts comes from round the tower, and
// the tilt is the shot's own dutch angle (none in a wide or a top shot).
fn choose_view() {
  let v = shot.index;
  let side = select(-1.0, 1.0, random(5u) < 0.5);
  // How far below the tip the camera is (less than 0 above it), how far from the axis, how far
  // below the tip it aims (the three views from below look up at about 29 degrees, which is to be
  // kept: raise one by taking the same off `below` and `aim`), how fast it drifts round, its field
  // of view and how much of the tower under the tip it must see.
  state.fov = 1.0;
  state.deep = 0.0;
  if (v == 2u) {
    state.below = 18.0;
    state.dist = 12.0 + 2.4 * params.girth;
    state.aim = 6.0;
    state.spin = 0.03 * side;
  } else if (v == 1u) {
    state.below = 10.0;
    state.dist = 8.0 + 1.6 * params.girth;
    state.aim = 2.0;
    state.spin = 0.04 * side;
  } else if (v == 3u) {
    // About 30 degrees down on the tip, from outside one of the tower's corners (the plates lie
    // on a square grid: a corner shows two faces, a side only one), drifting round slowly.
    state.below = -7.0;
    state.dist = 12.0 + 2.2 * params.girth;
    state.aim = 6.0;
    state.spin = 0.01 * side;
    state.azimuth = (floor(state.azimuth / 1.5708) + 0.5) * 1.5708;
    state.deep = 45.0;
  } else if (v == 4u) {
    state.below = 4.0;
    state.dist = 7.0 + params.girth;
    state.aim = 1.0;
    state.spin = 0.05 * side;
  } else if (v == 5u) {
    state.below = 7.0;
    state.dist = 6.0 + 1.2 * params.girth;
    state.aim = 0.0;
    state.spin = 0.05 * side;
  } else if (v == 6u) {
    state.below = 1.5;
    state.dist = 3.0 + 0.8 * params.girth;
    state.aim = 1.5;
    state.spin = 0.05 * side;
    state.fov = 0.6;
  } else if (v == 7u) {
    // About 40 degrees down, high over the tip and far off.
    state.below = -16.0;
    state.dist = 20.0 + 3.0 * params.girth;
    state.aim = 10.0;
    state.spin = 0.02 * side;
    state.deep = 60.0;
  } else {
    state.below = 14.0;
    state.dist = 10.0 + 2.0 * params.girth;
    state.aim = 4.0;
    state.spin = 0.05 * side;
  }
  // The burst's cuts come from round the tower, each from a new side.
  if (shot.fill > 0u) {
    state.azimuth += 2.2 * side;
  }
  state.view = v;
  state.tilt = shot.roll;
  state.cuts = shot.cuts;
}

// How much darker or brighter a light is exposed: the high sun a little darker, the blue hour,
// lit by the rod alone, brighter.
fn theme_exposure(m: f32) -> f32 {
  if (abs(m - 1.0) < 0.5) {
    return -0.2;
  }
  if (abs(m - 2.0) < 0.5) {
    return 0.6;
  }
  return 0.0;
}

// The layers whose middle the tip grew past, from `below` to `above`, take how full the music is
// now: a layer keeps it once grown, as its plates never change.
fn remember_fill(below: f32, above: f32, fill: f32) {
  let first = floor(below / CELL.y - 0.5) + 1.0;
  let last = min(floor(above / CELL.y - 0.5), first + f32(FILL_LAYERS) - 1.0);
  for (var y = first; y <= last; y += 1.0) {
    let i = fill_slot(y);
    state.fills[i / 4u][i % 4u] = fill;
  }
}

fn update() {
  let dt = rv_dt();
  if (frame.first == 1u) {
    state.top = 40.0;
    state.top_to = 40.0;
    state.lens = 1.0;
    state.per_beat = params.grow;
    // The tower that stands at the start is as full as the plates' density says.
    for (var i = 0; i < FILL_LAYERS / 4; i += 1) {
      state.fills[i] = vec4f(0.65);
    }
    state.azimuth = random(4u) * 6.283;
    choose_view();
    state.cam_top = state.top;
    state.cam_rise = 0.0;
    state.top_prev = state.top;
  }

  // Without a beat the hits of the music stand in for it.
  let beat = input.beat > 0.5;
  let onset = input.beat > 0.5 && input.tick < 0.5;

  // The kick: every hit, and between them the kicks of the music, while the beat is followed.
  let kin = max(f32(beat), params.hits * input.kick * input.locked);
  state.kick = max(state.kick * exp(-dt / params.kick_decay), kin);

  // How much it grows on a beat: by how fast the music is. input.tempo over 90 bpm is the pace,
  // so `grow` is the growth at 90 bpm; the louder the section (input.drive), the more.
  let pace = clamp(input.tempo / 90.0, 0.25, 3.0);
  state.per_beat = params.grow * pace * (0.7 + 0.6 * input.drive);

  // On every beat the tip shoots up, on a bar's first beat further, and every plate it reaches
  // appears on that beat. Without a beat it grows the same way on the hits, at most twice a
  // second at the usual pace.
  let tick = input.tick > 0.5 || (onset && state.since_beat > 0.5 / pace);
  if (tick) {
    let boost = select(1.0, params.bar_boost, input.tick > 0.5 && input.bar_line > 0.5);
    let step = max(round(state.per_beat * boost), 1.0) * CELL.y;
    state.top_prev = state.top_to;
    state.top_to += step;
    remember_fill(state.top_prev, state.top_to, input.fill);
    state.since_beat = 0.0;
  }
  state.since_beat += dt;
  state.top += (state.top_to - state.top) * (1.0 - exp(-dt / max(params.snap, 0.01)));

  // The rod breathes with the energy (declared `rod`, smoothed over a beat). Flashes and glitches
  // are the runtime's shared effects.
  state.rod = input.rod;

  // The view changes when the director cuts (`shot.cuts`). The light is the runtime's: it
  // changes the look on a section start by the section's label, and `look` says which and how
  // far the fade is.
  if (shot.cuts != state.cuts) {
    choose_view();
  }
  set_theme();

  // It circles at the tempo while the music is heard, and comes to rest in silence.
  state.azimuth += dt * state.spin * (0.6 + 0.4 * pace) * input.audible;

  // The camera rises after the tip, a little behind, so every beat's growth runs ahead of it,
  // looking up the tower's body towards the tip. It follows as a damped spring, so its speed
  // changes smoothly: it never lurches up on a beat.
  let a = state.azimuth;
  let w = 2.0 / max(params.follow, 0.1);
  state.cam_rise += (w * w * (state.top - state.cam_top) - 2.0 * w * state.cam_rise) * dt;
  state.cam_top += state.cam_rise * dt;
  let out = vec2f(cos(a), sin(a));
  state.cam_pos = vec3f(out.x * state.dist, state.cam_top - state.below, out.y * state.dist);
  state.look = vec3f(out.x * state.dist * 0.1, state.cam_top - state.aim, out.y * state.dist * 0.1);
  state.roll = state.tilt;
  state.lens = state.fov * (1.0 - params.zoom * state.kick);
  // In focus: the tower's core for the views from below, and what the camera looks at for the
  // others.
  state.focus = select(distance(state.look, state.cam_pos), state.dist * 0.85, state.view <= 2u);

  // Only the sun glares (`glow`); the lens is a clean one (scene.toml's [post]).
  post.bloom_intensity = params.glow;
  let now = looks_now();
  let dim = mix(theme_exposure(now.x), theme_exposure(now.y), now.z);
  post.exposure = params.exposure + dim;
}
