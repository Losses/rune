// Once per frame: glide down the street with the beat, cut at sections and bars, shake only at
// the big moments.

const DEG: f32 = 0.0174533;

fn random(n: u32) -> f32 {
  return rv_unit(frame.seed * 2654435761u + n);
}

// Pick a view: street level, aside looking across, above the signs, or high over the city.
// With `label` >= 0 (a section start) the view and the district's colours come from the label,
// so every chorus is seen the same way; otherwise they are random.
fn cut(label: f32) {
  let styled = label >= 0.0 && params.same_view == 1u;
  let k = label + 1.0;
  let r1 = select(random(1u), fract(k * 0.6180339 + 0.137), styled);
  let r2 = select(random(2u), fract(k * 0.7548777 + 0.291), styled);
  let r3 = select(random(3u), fract(k * 0.5698403 + 0.733), styled);
  let sg = select(-1.0, 1.0, r3 < 0.5);
  var view = 0.0;
  if (r2 * 100.0 < params.overview) {
    view = 3.0;
  } else if (r2 * 100.0 < params.overview + params.high) {
    view = 2.0;
  } else if (r1 * 100.0 < params.side) {
    view = 1.0;
  }
  state.view = view;
  state.side = f32(view == 1.0);
  if (view == 3.0) {
    state.eye = params.overview_height * (0.85 + 0.3 * r1);
    state.lat = (r3 - 0.5) * 6.0;
    state.yaw_base = (r1 - 0.5) * 30.0 * DEG;
    state.pitch_base = -params.overview_pitch * DEG;
  } else if (view == 2.0) {
    state.eye = params.high_eye;
    state.lat = (r3 - 0.5) * 1.6;
    state.yaw_base = (r1 - 0.5) * 8.0 * DEG;
    state.pitch_base = -0.26;
  } else if (view == 1.0) {
    state.eye = params.eye;
    state.lat = sg * (1.0 + 0.4 * r2);
    state.yaw_base = sg * (params.yaw + r2 * params.yaw_var) * DEG;
    state.pitch_base = 0.04 + 0.04 * r3;
  } else {
    state.eye = params.eye;
    state.lat = (r3 - 0.5) * 1.2;
    state.yaw_base = (r1 - 0.5) * 6.0 * DEG;
    state.pitch_base = 0.03 + 0.05 * r3;
  }
  if (label >= 0.0 || random(5u) < 0.3) {
    state.district = select(random(6u), fract(k * 0.381966 + 0.05), styled);
  }
  // Somewhere else along the street.
  state.pos.z = random(4u) * 2000.0;
  state.last_cut = frame.time;
  state.flash = 1.0;
}

// ---- The theme colour ----
// The player's accent colour (or the one set), as four tones of its hue made in OKLab, for the
// palette that maps the picture onto it as the reds do.

fn oklab(c: vec3f) -> vec3f {
  let l = pow(max(0.4122214708 * c.r + 0.5363325363 * c.g + 0.0514459929 * c.b, 0.0), 1.0 / 3.0);
  let m = pow(max(0.2119034982 * c.r + 0.6806995451 * c.g + 0.1073969566 * c.b, 0.0), 1.0 / 3.0);
  let s = pow(max(0.0883024619 * c.r + 0.2817188376 * c.g + 0.6299787005 * c.b, 0.0), 1.0 / 3.0);
  return vec3f(0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s, 1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s, 0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s);
}

fn oklch(lightness: f32, chroma: f32, hue: f32) -> vec4f {
  let a = chroma * cos(hue);
  let b = chroma * sin(hue);
  let l = pow(lightness + 0.3963377774 * a + 0.2158037573 * b, 3.0);
  let m = pow(lightness - 0.1055613458 * a - 0.0638541728 * b, 3.0);
  let s = pow(lightness - 0.0894841775 * a - 1.2914855480 * b, 3.0);
  return vec4f(max(vec3f(4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s, -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s, -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s), vec3f(0.0)), 0.0);
}

fn set_theme() {
  let follow = params.player_colour == 1u && frame.accent.a > 0.5;
  let theme = oklab(select(params.theme.rgb, frame.accent.rgb, follow));
  let h = atan2(theme.z, theme.y);
  state.theme_ramp[0] = oklch(0.12, 0.03, h);
  state.theme_ramp[1] = oklch(0.36, 0.11, h);
  state.theme_ramp[2] = oklch(0.62, 0.16, h);
  state.theme_ramp[3] = oklch(0.95, 0.03, h);
}

fn update() {
  let dt = frame.dt;
  let t = frame.time;
  if (frame.first == 1u) {
    state.vel = params.cruise;
    state.vtarget = params.cruise;
    state.tempo = 1.0;
    state.ens = 1.0;
    state.lens = 1.0;
    state.beat_gap = 0.5;
    state.district = random(7u);
    cut(-1.0);
    state.flash = 0.0;
  }

  let sure = music.bpm > 0.0 && music.beat_confidence > params.conf;
  let beat = f32(music.beats > 0u && sure);
  if (music.beats > 0u) {
    state.beat_gap = min(t - state.beat_time, 2.0);
    state.beat_time = t;
    state.beat_count += f32(music.beats);
    state.changes += f32(music.beats);
    if (sure) {
      state.sway_v += params.swing * 6.0 * select(-1.0, 1.0, fract(state.beat_count * 0.5) < 0.25);
    }
  }
  // The lanterns swing back on their strings, slowly coming to rest.
  state.sway_v += (-36.0 * state.sway - 3.0 * state.sway_v) * dt;
  state.sway += state.sway_v * dt;
  // The traffic lights change every four bars.
  if (music.bars > 0u && u32(floor(music.bar)) % 4u == 0u) {
    state.signal = 1.0 - state.signal;
  }

  // Energy: the section's, leaning towards the next one in the bars before it.
  let chart = music.has_chart == 1u;
  if (chart && music.bpm > 0.0) {
    let bar_seconds = music.beats_per_bar * 60.0 / music.bpm;
    state.ramp = clamp(1.0 - music.section_left / (max(params.build_bars, 0.25) * bar_seconds), 0.0, 1.0);
    state.energy = music.section_energy + (music.next_energy - music.section_energy) * state.ramp * state.ramp;
    state.rising = f32(music.next_energy > music.section_energy + 0.1) * state.ramp;
  } else {
    state.ramp = 0.0;
    state.rising = 0.0;
    state.energy += (music.loudness - state.energy) * (1.0 - exp(-dt / 1.5));
  }
  let en = 0.5 + 1.5 * state.energy;
  state.ens += (en - state.ens) * (1.0 - exp(-dt / 0.4));
  state.dark = f32(chart) * params.dark_quiet * pow(max(1.0 - state.energy, 0.0), 1.5);

  // The kick on every beat (glow, brightness, a small zoom), stronger when the music is loud.
  let kin = max(beat, params.hits * music.low * f32(sure));
  let level = select(min(music.energy / 1.5, 1.0), music.loudness * music.loudness, chart);
  let gain = max(1.0 + params.shake_energy * (1.25 * level - 1.0), 0.0);
  state.kick = max(state.kick * exp(-dt / params.kick_decay), kin * gain);

  // Accents shake the camera, and seldom: a section that lifts the energy, or in a loud part a
  // hit far above the usual, at least shake_gap seconds apart.
  var accent = 0.0;
  if (chart && music.sections > 0u && music.section_energy > state.last_energy + 0.1) {
    accent = 1.0;
  }
  if (music.sections > 0u) {
    state.last_energy = music.section_energy;
  }
  let loud = select(music.energy > 1.2, state.energy > 0.55, chart);
  if (loud && music.low > params.accent_level && t - state.last_accent > params.shake_gap) {
    accent = max(accent, 0.7 * music.low);
  }
  if (accent > 0.0) {
    state.last_accent = t;
    state.accent = max(state.accent, accent * gain);
  }
  state.accent *= exp(-dt / params.shake_decay);
  state.shake = 0.012 * state.accent * params.shake;

  // Cuts: at a section's start in its own view; on bar lines now and then.
  let gap_ok = t - state.last_cut > params.cut_gap;
  if (chart && music.sections > 0u && t - state.last_cut > 0.5) {
    cut(music.section_label);
  } else if (music.bars > 0u && gap_ok && (!chart || music.meter_confidence > 0.3)) {
    let bar = u32(floor(music.bar));
    let every = max(u32(params.cut_bars), 1u);
    let chance = params.cut_chance * max(1.0 + (state.energy - 0.5), 0.0);
    if (bar % every == 0u && random(9u) * 100.0 < chance) {
      cut(-1.0);
    }
  } else if (music.bpm <= 0.0 && t - state.last_cut > 12.0) {
    cut(-1.0);
  }
  state.flash *= exp(-dt / 0.12);

  // Gliding as fast as the music sounds (music.feel, about 3 Hz for a ballad to 7 for drum and
  // bass; 5 is cruise): the song's own pace, and stretched further, how much faster or slower
  // this moment is than the song, so a song's quiet and loud parts look as different as they
  // sound. A push on every beat; faster up high.
  let song = select(5.0, music.song_feel, music.song_feel > 0.0);
  let here = select(song, music.feel, music.feel > 0.0);
  if (state.feel <= 0.0) {
    state.feel = here;
  }
  state.feel += (here - state.feel) * (1.0 - exp(-dt / params.feel_smooth));
  let pace = pow(song / 5.0, params.song_pace) * pow(state.feel / song, params.moment_pace);
  state.tempo += (clamp(pace, 0.25, 3.0) - state.tempo) * (1.0 - exp(-dt / 1.0));
  let height = 1.0 + max(state.eye - params.eye, 0.0) / 12.0;
  let cruise = params.cruise * height * state.tempo * (1.0 + params.energy_speed * (state.ens - 1.0));
  state.vtarget = cruise + (state.vtarget - cruise) * exp(-dt / params.ease) + beat * params.push * state.tempo;
  state.vel += (state.vtarget - state.vel) * (1.0 - exp(-dt / params.accel));
  state.pos.z += dt * state.vel;
  if (state.pos.z > 4000.0) {
    state.pos.z -= 3000.0;
  }

  let wobble = vec2f(sin(t * 23.1), sin(t * 19.3 + 0.7));
  state.pos.x = state.lat;
  state.pos.y = state.eye;
  state.yaw = state.yaw_base + 0.01 * sin(t * 0.17) + state.shake * wobble.x;
  state.pitch = state.pitch_base + 0.008 * sin(t * 0.13) + state.shake * wobble.y;
  state.roll = 0.004 * sin(t * 0.23) + state.side * 0.015 * sin(t * 0.31) + state.shake * 0.5 * wobble.y;
  state.lens = (1.0 - params.zoom * state.kick) * (1.0 - 0.25 * params.build * state.rising * state.ramp);

  set_theme();
  fill_rows();
  // The look follows too: more glow when it is loud and on the kick, a flash at a cut.
  post.bloom_intensity = params.glow * (0.7 + 0.6 * state.energy) + params.glow_beat * state.kick;
  post.flash = state.flash * params.flash * 0.5;
  post.chromatic = 0.0012 * params.chroma * (1.0 + 2.0 * state.kick + 4.0 * state.accent);
  post.exposure = params.exposure + 0.15 * params.pump * state.kick;
}

// What the rows ahead of the camera hold (see State): what every pixel would otherwise work out
// for itself, row by row.
fn fill_rows() {
  let r0 = floor(state.pos.z / UNIT) - 1.0;
  state.row0 = r0;
  for (var i = 0u; i < ROWS; i++) {
    let n = r0 + f32(i);
    for (var k = 0u; k < 2u; k++) {
      let s = f32(k) * 2.0 - 1.0;
      let row = i * 2u + k;
      state.row_sign[row] = sign_of(n, s);
      state.row_shop[row] = vec4f(wall_x(n, s), furniture(n, s), f32(awning_on(n, s)), f32(pole_on(n)));
      state.row_front[row] = vec4f(first_row(n, s), rows_of(n, s), height_of(i32(s), n), front_kind(n, s));
    }
    for (var k = 0u; k < 3u; k++) {
      for (var j = 0u; j < 2u; j++) {
        var who = person(person_in_row(n, f32(k), f32(j)), f32(k));
        if (floor(who.y / UNIT) != n) {
          who = vec4f(0.0);
        }
        state.row_people[i * 6u + k * 2u + j] = who;
      }
    }
  }
}
