// Once per frame: glide down the street at the music's tempo, follow the director's cuts, shake
// only at the big moments.

const DEG: f32 = 0.0174533;

// How long a push on the scroll's speed takes to ease off, seconds.
const SCROLL_EASE: f32 = 0.25;

// A number from 0 to 1 for this cut, `n` telling the uses apart: made from the shot's seed from
// the runtime's director, which is fixed within a shot and new at each cut.
fn random(n: u32) -> f32 {
  return rv_unit(u32(shot.seed * 16777215.0) * 2654435761u + n);
}

// Look from `eye` at `at`: the yaw and pitch (a positive yaw looks towards -x, a positive pitch
// up).
fn aim(eye: vec3f, at: vec3f) -> vec2f {
  let d = at - eye;
  return vec2f(atan2(-d.x, d.z), atan2(d.y, length(d.xz)));
}

// The burst's close shots: put its subject in row n on side s (see hero_row in state.wgsl) and
// frame it as suits its size, from where the camera stands and what it looks at; `back` stands
// that many times as far off along the ground (at the same height), for the wider of the two.
// The camera creeps on through the shot (update), a little nearer by its end.
fn frame_hero(kind: f32, n: f32, s: f32, back: f32) {
  state.hero = vec4f(kind, n, s, 1.0);
  let z = n * UNIT;
  let wall = wall_x(n, s);
  var eye: vec3f;
  var at: vec3f;
  if (kind == HERO_TORII) {
    // From beside its pillar on side s (2.2 m out), looking up past it along the beams going away
    // across the street (the kasagi 6.6 m up), the far side's fronts behind them. (Looking up the
    // street instead, into the open sky, cost a fifth more: those rays walk the grid far.)
    eye = vec3f(s * 1.6, 1.4, z + 2.25 - 1.8);
    at = vec3f(-s * 0.6, 4.2, z + 2.25);
  } else if (kind == HERO_BIKE) {
    // A metre and a bit from its basket, low, in front and to one side.
    let front = s * (wall - 1.6);
    eye = vec3f(front - s * 1.0, 0.75, z + 2.25 - 0.9);
    at = vec3f(front + s * 0.1, 0.72, z + 2.25);
  } else if (kind == HERO_SIGNAL) {
    // From the road below, across the street from its pole, up at the lamps over the road (the
    // head hangs 2.4 m in from the kerb, 5 m up), the fronts of its side behind it (as for the
    // torii: not up the street into the open sky).
    let head = vec3f(s * (HALF - 2.75), 5.15, z + 1.2);
    eye = vec3f(-s * 0.9, 1.5, head.z - 2.0);
    at = head - vec3f(0.0, 0.4, 0.0);
  } else if (kind == HERO_LAMP) {
    // At the foot of its pole, looking up along it to the globes.
    let pole = vec3f(s * (HALF - 0.4), 0.0, z + 2.25);
    eye = vec3f(pole.x - s * 1.3, 1.3, pole.z - 2.5);
    at = vec3f(pole.x, 3.9, pole.z);
  } else if (kind == HERO_VENDING) {
    // Three-quarter on to their lit fronts, at the height of the bottles.
    let mid = z + 0.5 + 0.475 * vendings_of(n, s);
    eye = vec3f(s * (wall - 2.5), 1.35, mid - 1.9);
    at = vec3f(s * (wall - 0.75), 1.2, mid + 0.1);
  } else {
    // Low, at the shrine's roof and its lanterns.
    let mid = z + 2.25;
    eye = vec3f(s * (wall - 2.5), 0.7, mid - 1.7);
    at = vec3f(s * (wall - 1.0), 1.15, mid);
  }
  eye = vec3f(at.xz + (eye.xz - at.xz) * back, eye.y).xzy;
  let look = aim(eye, at);
  state.lat = eye.x;
  state.eye = eye.y;
  state.pos.z = eye.z;
  state.yaw_base = look.x;
  state.pitch_base = look.y;
}

// Frame the shot the director cut to: street level, the street side, above the signs, or high
// over the city (the director chooses which and when, see `[[shot]]` in scene.toml), or one of
// the burst's four (shot.index 4 to 7, shot.fill 1 to 4): street level looking ahead, close on a
// subject, closer on another, and high up looking down the street. Each burst view jumps to a
// new place along the street, the subjects put a little ahead. The district's colours may change
// at the cut: at a section start they come from the section's label, so every chorus is seen in
// the same colours; at any other cut they change now and then, by chance. (The look's grey or
// theme colour is the runtime's: see grade.wgsl.)
fn cut() {
  let label = select(-1.0, input.label, input.cue_mid > 0.5);
  let styled = label >= 0.0 && params.same_view == 1u;
  let k = label + 1.0;
  let r1 = random(1u);
  let r2 = random(2u);
  let r3 = random(3u);
  let sg = select(-1.0, 1.0, r3 < 0.5);
  let view = f32(min(shot.index, 7u));
  let was = state.view;
  let before = state.hero;
  state.view = view;
  // Somewhere else along the street, and the street as it is (the burst's subject goes).
  state.pos.z = random(4u) * 2000.0;
  state.hero = vec4f(0.0);
  if (view == 7.0) {
    // High, looking down the street (the overview shot is higher and steeper).
    state.eye = 20.0;
    state.lat = (r3 - 0.5) * 2.0;
    state.yaw_base = (r1 - 0.5) * 10.0 * DEG;
    state.pitch_base = -0.62;
  } else if (view == 5.0 || view == 6.0) {
    // Close on a subject (5), then closer on another (6): a torii over the street, a bicycle, a
    // traffic light, a street lamp, vending machines or a little shrine, by the shot's seed, put in
    // a row a little ahead (not at a cross street or an arch). The second is never the kind the
    // first was (the two must differ).
    var n = floor(state.pos.z / UNIT) + 3.0;
    for (var i = 0; i < 3; i++) {
      if (crossing(n) || arch_on(n)) {
        n += 1.0;
      }
    }
    var kind = min(floor(random(8u) * HERO_KINDS), HERO_KINDS - 1.0);
    if (view == 6.0 && was == 5.0 && before.w > 0.5) {
      let other = min(floor(random(8u) * (HERO_KINDS - 1.0)), HERO_KINDS - 2.0);
      kind = (before.x + 1.0 + other) % HERO_KINDS;
    }
    frame_hero(kind, n, sg, select(1.0, 1.35, view == 5.0));
  } else if (view == 4.0) {
    // Street level a little to one side, looking ahead down the street.
    state.eye = 2.2;
    state.lat = sg * (0.6 + 0.8 * r2);
    state.yaw_base = -sg * (1.0 + 3.0 * r1) * DEG;
    state.pitch_base = 0.04;
  } else if (view == 3.0) {
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
    // The street side: the burst's first view (street level a little to one side, looking ahead
    // down the street) turned towards the shops on the other side, by `yaw` and its spread, so
    // their fronts and signs fill that side of the frame while the street leads away on the
    // other. The turn leaves the street's far end 12 degrees inside the frame's edge: less on a
    // narrow (portrait) screen, whose view is narrower.
    let half = atan(0.72 * frame.aspect);
    let turn = clamp(min((params.yaw + r2 * params.yaw_var) * DEG, half - 12.0 * DEG), 8.0 * DEG, 45.0 * DEG);
    state.eye = 2.2;
    state.lat = -sg * (0.6 + 0.8 * r1);
    state.yaw_base = -sg * turn;
    state.pitch_base = 0.06;
  } else {
    state.eye = params.eye;
    state.lat = (r3 - 0.5) * 1.2;
    state.yaw_base = (r1 - 0.5) * 6.0 * DEG;
    state.pitch_base = 0.03 + 0.05 * r3;
  }
  if (label >= 0.0 || random(5u) < 0.3) {
    state.district = select(random(6u), fract(k * 0.381966 + 0.05), styled);
  }
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
  state.theme_ramp[1] = oklch(0.36, 0.14, h);
  state.theme_ramp[2] = oklch(0.62, 0.19, h);
  state.theme_ramp[3] = oklch(0.95, 0.03, h);
}

fn update() {
  let dt = rv_dt();
  let t = frame.time;
  if (frame.first == 1u) {
    state.vel = params.cruise;
    state.vtarget = params.cruise;
    state.lens = 1.0;
    state.ring_gap = 0.5;
    state.district = random(7u);
    state.cuts = shot.cuts;
    cut();
    state.clear = 0.0;
  }

  // Without a beat the hits of the music stand in for it. A hit counts for the things below only
  // while something is heard: a stray onset in near silence (`audible` under 0.3) moves nothing.
  // `tick` is a beat the runtime follows (level A); `beat_s` is a beat's length in seconds.
  let beat = input.beat;
  let heard = beat > 0.5 && input.audible > 0.3;
  let beat_s = 0.667 / input.pace;
  if (heard) {
    // Another step for the signs' patterns and the big screens' modes, a smaller one in a fade
    // (so they come slower), and the big screen's ring starts.
    state.changes += input.audible;
    state.ring_gap = min(t - state.ring_time, 2.0);
    state.ring_time = t;
  }
  // The lanterns swing to a followed beat, a side and then the other. Without one the hits do it,
  // only 0.6 as hard and at most every other beat (onsets can be dense), alternating all the same
  // (the lanterns at the other levels too).
  let followed = input.tick > 0.5;
  if (followed || (heard && t - state.sway_time > 2.0 * beat_s)) {
    state.sway_n += 1.0;
    state.sway_time = t;
    state.sway_v += params.swing * 6.0 * select(0.6, 1.0, followed) * select(-1.0, 1.0, fract(state.sway_n * 0.5) < 0.25);
  }
  // The lanterns swing back on their strings, slowly coming to rest.
  state.sway_v += (-36.0 * state.sway - 3.0 * state.sway_v) * dt;
  state.sway += state.sway_v * dt;

  // The boards and the bands scroll on at the song's pace, a beat's worth in a beat on average:
  // 0.6 of it steadily and the rest as a push on each beat (a followed one, or a heard hit at
  // most once in 0.9 beats' time), the speed easing off after it as the camera's does. Both are
  // times `audible`, so a fade slows the scroll and the weaker pushes, and in silence it stands
  // still (no jump a step on the beat, and still in silence).
  let scroll_rate = 1.5 * input.pace * input.audible;
  state.scroll_t = 0.6 * scroll_rate + (state.scroll_t - 0.6 * scroll_rate) * exp(-dt / SCROLL_EASE);
  if (followed || (heard && t - state.scroll_time > 0.9 * beat_s)) {
    state.scroll_t += 0.4 * input.audible / SCROLL_EASE;
    state.scroll_time = t;
  }
  state.scroll_v += (state.scroll_t - state.scroll_v) * (1.0 - exp(-dt / 0.1));
  // Kept under 4096, a multiple of every period the scroll is read in.
  state.scroll = (state.scroll + state.scroll_v * dt) % 4096.0;

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

  // Accents shake the camera, and seldom: a cue that lifts the energy (strength 1), or a hard hit
  // in the middle of a section (the runtime keeps those at least 6 s apart).
  state.accent = max(input.lift_env, 0.7 * input.impact_env) * input.gain;
  state.shake = 0.012 * state.accent * params.shake;

  // The director cuts: take its shot as the view. The trail is cleared across the cut.
  if (shot.cuts != state.cuts) {
    state.cuts = shot.cuts;
    cut();
  }
  state.clear *= exp(-dt / 0.12);

  // Gliding as fast as the music: the tempo over 90 bpm is the pace, so a cruise is the speed at 90
  // bpm, faster for a faster song and slower for a slower one. Down low it is faster and swings
  // more with the music, up high slower and steadier (both by the music): from `cruise` at 3 m to
  // `cruise_high` at 45 m by the height's logarithm, a quiet passage at half speed to a loud one
  // 1.3 times down low, 0.6 to 1.1 times up high (`energy`), the push on the beat a third as strong
  // up high. A fade slows it and silence nearly stills it (`audible`).
  let pace = input.pace;
  let up = clamp(log(max(state.eye, 0.1) / 3.0) / log(15.0), 0.0, 1.0);
  let swing = mix(vec2f(0.5, 1.3), vec2f(0.6, 1.1), up);
  let by_energy = mix(1.0, mix(swing.x, swing.y, input.energy), params.energy_speed);
  let slow = pace * by_energy * mix(0.2, 1.0, input.audible);
  let cruise = mix(params.cruise, params.cruise_high, up) * slow;
  let push = params.push * mix(1.0, 1.0 / 3.0, up) * slow;
  state.vtarget = cruise + (state.vtarget - cruise) * exp(-dt / params.ease) + beat * push;
  state.vel += (state.vtarget - state.vel) * (1.0 - exp(-dt / params.accel));
  // Close on a burst's subject the camera only creeps, or it would be past it within the beat.
  state.pos.z += dt * state.vel * select(1.0, 0.2, state.hero.w > 0.5);
  if (state.pos.z > 4000.0) {
    state.pos.z -= 3000.0;
  }

  let wobble = vec2f(sin(t * 23.1), sin(t * 19.3 + 0.7));
  state.pos.x = state.lat;
  state.pos.y = state.eye;
  state.yaw = state.yaw_base + 0.01 * sin(t * 0.17) + state.shake * wobble.x;
  state.pitch = state.pitch_base + 0.008 * sin(t * 0.13) + state.shake * wobble.y;
  // The camera never rolls of its own accord: the director's dutch angle (only in a burst) and
  // the shake at an accent are all there is.
  state.roll = shot.roll + state.shake * 0.5 * wobble.y;
  state.lens = 1.0 - 0.25 * params.build * rising;

  set_theme();
  fill_rows();
  // The looks: 0 grey, 1 the theme colour. How far the picture is in the colour one now, as the
  // runtime fades from look.prev to look.to (grade.wgsl draws both).
  state.colour = select(0.0, 1.0 - look.mix, look.prev == 1u) + select(0.0, look.mix, look.to == 1u);
  // The look follows too: more glow when it is loud (`bloom`), and a dimmer one in grey.
  post.bloom_intensity = input.bloom * mix(0.7, 1.0, state.colour);
  // The accent shakes the camera and nothing else: the chromatic aberration stays as it is.
  post.chromatic = 0.0012 * params.chroma;
  post.exposure = params.exposure;
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
