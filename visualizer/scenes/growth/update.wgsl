// Once per frame: the tower shoots up on every beat (more on a bar's first), the camera rising
// after it and circling; an accent lights the rod and washes the picture white; each section is
// seen its own way. A plate once grown never changes.

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

// The colours now: the last theme fading into the next; in the player's colour if it follows it
// and the player has told it.
fn set_theme() {
  let a = theme_of(state.theme_from);
  let b = theme_of(state.theme_to);
  let k = smoothstep(0.0, 1.0, state.theme_mix);
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

// Pick the view to glide to. Every view looks up the tower from below (so what is under the
// camera never needs drawing): 0 close and steeply tilted, 1 closer, tilted the other way,
// 2 further off and nearly level. With `label` >= 0 (a section start) it comes from the label,
// so every chorus is seen the same way.
fn choose_view(label: f32, fresh: bool) {
  let styled = label >= 0.0 && params.same_view == 1u;
  let k = label + 1.0;
  let r1 = select(random(1u), fract(k * 0.6180339 + 0.137), styled);
  let r2 = select(random(2u), fract(k * 0.7548777 + 0.291), styled);
  var v = floor(r1 * 3.0);
  // A random choice never stays where it is.
  if (!styled && !fresh && v == state.view) {
    v = f32((u32(v) + 1u + u32(r2 * 2.0)) % 3u);
  }
  state.view = v;
  let side = select(-1.0, 1.0, r2 < 0.5);
  // How far below the tip the camera is, how far from the axis, how far below the tip it aims,
  // its tilt and how fast it drifts round.
  if (v > 1.5) {
    state.below_to = 34.0;
    state.dist_to = 12.0 + 2.4 * params.girth;
    state.aim_to = 22.0;
    state.tilt_to = -0.25 * side;
    state.spin_to = 0.03 * side;
  } else if (v > 0.5) {
    state.below_to = 18.0;
    state.dist_to = 8.0 + 1.6 * params.girth;
    state.aim_to = 10.0;
    state.tilt_to = 0.45 * side;
    state.spin_to = 0.04 * side;
  } else {
    state.below_to = 26.0;
    state.dist_to = 10.0 + 2.0 * params.girth;
    state.aim_to = 16.0;
    state.tilt_to = -0.55 * side;
    state.spin_to = 0.05 * side;
  }
  state.last_view = frame.time;
}

// How much darker or brighter a theme is exposed: midnight is a dark theme, which the tone mapping
// would lift to grey at the bright themes' exposure; concrete's blue hour is lit by the rod alone.
fn theme_exposure(m: f32) -> f32 {
  if (abs(m - 2.0) < 0.5) {
    return -1.6;
  }
  if (abs(m - 6.0) < 0.5) {
    return -0.2;
  }
  if (abs(m - 7.0) < 0.5) {
    return 0.6;
  }
  return 0.0;
}

// Pick the theme to change to, by the section's label, so the same kind of section has the same
// colours (for concrete, the same light).
fn choose_theme(label: f32) {
  var m: f32;
  if (params.style == 1u) {
    if (params.light > 0u) {
      m = 4.0 + f32(params.light);
    } else if (label >= 0.0) {
      m = 5.0 + floor(fract(label * 0.381966 + 0.13) * 3.0);
    } else {
      m = 5.0 + f32((u32(max(state.theme_to - 5.0, 0.0)) + 1u + u32(random(14u) * 2.0)) % 3u);
    }
  } else if (params.theme > 0u) {
    m = f32(params.theme - 1u);
  } else if (label >= 0.0) {
    m = floor(fract(label * 0.381966 + 0.13) * 5.0);
  } else {
    m = f32((u32(state.theme_to) + 1u + u32(random(14u) * 4.0)) % 5u);
  }
  state.theme_next = m;
  state.last_theme = frame.time;
}

fn update() {
  let dt = frame.dt;
  let t = frame.time;
  if (frame.first == 1u) {
    state.top = 40.0;
    state.top_to = 40.0;
    state.lens = 1.0;
    state.ens = 1.0;
    state.per_beat = params.grow;
    state.azimuth = random(4u) * 6.283;
    choose_view(-1.0, true);
    choose_theme(-1.0);
    state.theme_from = state.theme_next;
    state.theme_to = state.theme_next;
    state.theme_mix = 1.0;
    state.below = state.below_to;
    state.dist = state.dist_to;
    state.aim = state.aim_to;
    state.cam_top = state.top;
    state.tilt = state.tilt_to;
    state.spin = state.spin_to;
    state.top_prev = state.top;
  }

  let sure = music.bpm > 0.0 && music.beat_confidence > params.conf;
  let beat = music.beats > 0u && sure;

  // Energy: the section's, leaning towards the next one in the bars before it.
  let chart = music.has_chart == 1u;
  if (chart && music.bpm > 0.0) {
    let bar_seconds = music.beats_per_bar * 60.0 / music.bpm;
    let ramp = clamp(1.0 - music.section_left / (max(params.build_bars, 0.25) * bar_seconds), 0.0, 1.0);
    state.energy = music.section_energy + (music.next_energy - music.section_energy) * ramp * ramp;
  } else {
    state.energy += (music.loudness - state.energy) * (1.0 - exp(-dt / 1.5));
  }
  let en = 0.5 + 1.5 * state.energy;
  state.ens += (en - state.ens) * (1.0 - exp(-dt / 0.4));
  let kin = max(f32(beat), params.hits * music.low * f32(sure));
  state.kick = max(state.kick * exp(-dt / params.kick_decay), kin);

  // How much it grows on a beat: by how fast the music sounds (as in Neon Street), the song's
  // own pace and, stretched further, this moment's against it.
  let song = select(5.0, music.song_feel, music.song_feel > 0.0);
  let here = select(song, music.feel, music.feel > 0.0);
  if (state.feel <= 0.0) {
    state.feel = here;
  }
  state.feel += (here - state.feel) * (1.0 - exp(-dt / params.feel_smooth));
  let pace = clamp(pow(song / 5.0, params.song_pace) * pow(state.feel / song, params.moment_pace), 0.25, 3.0);
  state.per_beat = params.grow * pace * (0.7 + 0.6 * state.energy);

  // On every beat the tip shoots up, on a bar's first beat further, and every plate it reaches
  // appears on that beat. Without a beat it grows the same way twice a second.
  let tick = beat || (!sure && state.since_beat > 0.5 / pace);
  if (tick) {
    let boost = select(1.0, params.bar_boost, beat && music.bars > 0u);
    let step = max(round(state.per_beat * boost), 1.0) * CELL.y;
    state.top_prev = state.top_to;
    state.top_to += step;
    state.since_beat = 0.0;
  }
  state.since_beat += dt;
  state.top += (state.top_to - state.top) * (1.0 - exp(-dt / max(params.snap, 0.01)));

  // Accents: a section that lifts the energy, or in a loud part a hit far above the usual.
  var accent = 0.0;
  if (chart && music.sections > 0u && music.section_energy > state.last_energy + 0.1) {
    accent = 1.0;
  }
  if (music.sections > 0u) {
    state.last_energy = music.section_energy;
  }
  let loud = select(music.energy > 1.2, state.energy > 0.55, chart);
  if (loud && music.low > params.accent_level && t - state.last_accent > params.accent_gap) {
    accent = max(accent, 0.75 * music.low);
  }
  if (accent > 0.0) {
    state.last_accent = t;
    state.flash = max(state.flash, accent);
  }
  state.flash *= exp(-dt / 0.35);
  // A glitch on an accent, and on the first beat of every few bars.
  let every = max(u32(params.glitch_bars), 1u);
  // Concrete keeps the beat in the growth alone: no glitch, no flash of the whole picture.
  let concrete = params.style == 1u;
  if (!concrete && (accent > 0.0 || (beat && music.bars > 0u && params.glitch_bars > 0.5 && u32(floor(music.bar)) % every == 0u))) {
    state.glitch = params.glitch;
  }
  state.glitch *= exp(-dt / 0.06);

  // Views: at a section's start by its label; otherwise now and then on a bar line.
  if (chart && music.sections > 0u && t - state.last_view > 2.0) {
    choose_view(music.section_label, false);
    choose_theme(music.section_label);
  } else if (music.bars > 0u && t - state.last_view > params.view_gap) {
    let bar = u32(floor(music.bar));
    if (bar % max(u32(params.view_bars), 1u) == 0u && random(9u) * 100.0 < params.view_chance) {
      choose_view(-1.0, false);
    }
  } else if (music.bpm <= 0.0 && t - state.last_view > 20.0) {
    choose_view(-1.0, false);
  }
  // Without a chart the theme changes every few bars.
  if (!chart && music.bars > 0u && t - state.last_theme > 8.0 && u32(floor(music.bar)) % max(u32(params.theme_bars), 1u) == 0u) {
    choose_theme(-1.0);
  } else if (music.bpm <= 0.0 && t - state.last_theme > 30.0) {
    choose_theme(-1.0);
  }
  // A change of style changes the theme at once.
  if (concrete != (state.theme_next > 4.5)) {
    choose_theme(-1.0);
  }
  if (state.theme_mix >= 1.0 && state.theme_next != state.theme_to) {
    state.theme_from = state.theme_to;
    state.theme_to = state.theme_next;
    state.theme_mix = 0.0;
  }
  state.theme_mix = min(state.theme_mix + dt / max(params.theme_fade, 0.05), 1.0);
  set_theme();

  let ease = 1.0 - exp(-dt / max(params.glide, 0.1));
  state.below += (state.below_to - state.below) * ease;
  state.dist += (state.dist_to - state.dist) * ease;
  state.aim += (state.aim_to - state.aim) * ease;
  state.tilt += (state.tilt_to - state.tilt) * ease;
  state.spin += (state.spin_to - state.spin) * ease;
  state.azimuth += dt * state.spin * (0.6 + 0.4 * pace);

  // The camera rises after the tip, a little behind, so every beat's growth runs ahead of it,
  // looking up the tower's body towards the tip.
  let a = state.azimuth;
  state.cam_top += (state.top - state.cam_top) * (1.0 - exp(-dt / max(params.follow, 0.05)));
  let out = vec2f(cos(a), sin(a));
  state.cam_pos = vec3f(out.x * state.dist, state.cam_top - state.below, out.y * state.dist);
  state.look = vec3f(out.x * state.dist * 0.1, state.cam_top - state.aim, out.y * state.dist * 0.1);
  state.roll = state.tilt + 0.01 * sin(t * 0.3);
  state.lens = 1.0 - params.zoom * state.kick;

  post.bloom_intensity = params.glow * (0.6 + 0.6 * state.energy) + params.glow_beat * state.kick + 1.2 * state.flash * params.flash;
  let dim = mix(theme_exposure(state.theme_from), theme_exposure(state.theme_to), smoothstep(0.0, 1.0, state.theme_mix));
  post.exposure = params.exposure + dim + 0.2 * params.pump * state.kick + 0.9 * state.flash * params.flash;
  if (concrete) {
    // Only the sun glares; the lens is a clean one.
    post.bloom_intensity = 0.25 * params.glow;
    post.exposure = params.exposure + dim;
    post.chromatic = 0.0;
    post.vignette = 0.25;
    post.grain = 0.02;
  }
}
