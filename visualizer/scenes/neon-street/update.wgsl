// Once per frame: glide down the street at the music's tempo, follow the director's cuts, shake
// only at the big moments.

const DEG: f32 = 0.0174533;

// A number from 0 to 1 for this cut, `n` telling the uses apart: made from the shot's seed from
// the runtime's director, which is fixed within a shot and new at each cut.
fn random(n: u32) -> f32 {
  return rv_unit(u32(shot.seed * 16777215.0) * 2654435761u + n);
}

// Frame the shot the director cut to: street level, aside looking across, above the signs, or
// high over the city (the director chooses which and when, see `[[shot]]` in scene.toml). The
// district's colours may change at the cut: at a section start they come from the section's label,
// so every chorus is seen in the same colours; at any other cut they change now and then, by
// chance.
fn cut() {
  let label = select(-1.0, input.label, input.cue_mid > 0.5);
  let styled = label >= 0.0 && params.same_view == 1u;
  let k = label + 1.0;
  let r1 = random(1u);
  let r2 = random(2u);
  let r3 = random(3u);
  let sg = select(-1.0, 1.0, r3 < 0.5);
  let view = f32(min(shot.index, 3u));
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
  // The trail is cleared at every cut (compose.wgsl).
  state.clear = 1.0;
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
  let dt = rv_dt();
  let t = frame.time;
  if (frame.first == 1u) {
    state.vel = params.cruise;
    state.vtarget = params.cruise;
    state.lens = 1.0;
    state.beat_gap = 0.5;
    state.district = random(7u);
    state.cuts = shot.cuts;
    cut();
    state.clear = 0.0;
  }

  // Without a beat the hits of the music stand in for it, except for the lanterns' swing, which
  // only follows a beat the runtime trusts (it counts beats only then).
  let beat = input.beat;
  state.beat_count = input.beat_no;
  state.changes = input.beat_no;
  if (input.beat > 0.5) {
    state.beat_gap = min(t - state.beat_time, 2.0);
    state.beat_time = t;
    if (input.tick > 0.5) {
      state.sway_v += params.swing * 6.0 * select(-1.0, 1.0, fract(state.beat_count * 0.5) < 0.25);
    }
  }
  // The lanterns swing back on their strings, slowly coming to rest.
  state.sway_v += (-36.0 * state.sway - 3.0 * state.sway_v) * dt;
  state.sway += state.sway_v * dt;
  // The traffic lights change every four bars (input.signal is 0 and 1 by turns, green first).
  state.signal = 1.0 - input.signal;

  // Energy: the runtime's slow one (the section's, leaning to the next in the bars before it).
  // How many signs are dark follows it too: the quieter the section, the more are dark.
  state.dark = params.dark_quiet * pow(max(1.0 - input.drive, 0.0), 1.5);

  // Build-up: when the next cue is known (cue_in) and the next section is louder, the lens pushes
  // in over the last build_bars bars before it, along a squared ramp.
  var rising = 0.0;
  if (input.cue_in >= 0.0 && input.next_energy > input.drive + 0.1) {
    let bar_seconds = 60.0 * input.meter / input.tempo;
    let ramp = clamp(1.0 - input.cue_in / (max(params.build_bars, 0.25) * bar_seconds), 0.0, 1.0);
    rising = ramp * ramp;
  }

  // The kick on every beat (glow, brightness, a small zoom), stronger when the music is loud.
  let gain = max(1.0 + params.shake_energy * (1.25 * input.intensity * input.intensity - 1.0), 0.0);
  state.kick = max(input.kick_env, params.hits * input.kick_soft) * gain;

  // Accents shake the camera, and seldom: a cue that lifts the energy (strength 1), or a hard hit
  // in the middle of a section (the runtime keeps those at least 6 s apart).
  state.accent = max(input.lift_env, 0.7 * input.impact_env) * gain;
  state.shake = 0.012 * state.accent * params.shake;

  // The director cuts: take its shot as the view. The trail is cleared across the cut.
  if (shot.cuts != state.cuts) {
    state.cuts = shot.cuts;
    cut();
  }
  state.clear *= exp(-dt / 0.12);

  // Gliding as fast as the music: the tempo over 90 bpm is the pace, so cruise is the speed at
  // 90 bpm, faster for a faster song and slower for a slower one. A push on every beat; faster
  // up high.
  let pace = clamp(input.tempo / 90.0, 0.25, 3.0);
  let energy_scale = 1.0 + params.energy_speed * (1.5 * input.drive - 0.5);
  let height = 1.0 + max(state.eye - params.eye, 0.0) / 12.0;
  let cruise = params.cruise * height * pace * energy_scale;
  state.vtarget = cruise + (state.vtarget - cruise) * exp(-dt / params.ease) + beat * params.push * pace;
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
  state.roll = 0.004 * sin(t * 0.23) + state.side * 0.015 * sin(t * 0.31) + shot.roll + state.shake * 0.5 * wobble.y;
  state.lens = (1.0 - params.zoom * state.kick) * (1.0 - 0.25 * params.build * rising);

  set_theme();
  fill_rows();
  // The look follows too: more glow when it is loud and on the kick.
  post.bloom_intensity = params.glow * (0.7 + 0.6 * input.drive) + params.glow_beat * state.kick;
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
