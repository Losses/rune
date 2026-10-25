// Once per frame: the beat clock the screens change channel by, the camera travelling along the
// wall as fast as the music sounds (pushed on each beat), the views changing at sections, the
// phosphor's colour from the player's, and the scopes' trigger and gain.

fn random(n: u32) -> f32 {
  return rv_unit(frame.seed * 2654435761u + n);
}

// Pick a view, and where on the wall to see it from. With `label` >= 0 (a section start) the view
// comes from the label, so every chorus is seen the same way. Views: 0 straight on, 1 from one
// side looking along the wall, 2 far off, 3 close up through a long lens.
fn choose_view(label: f32) {
  let styled = label >= 0.0 && params.same_view == 1u;
  let k = label + 1.0;
  let r1 = select(random(1u), fract(k * 0.6180339 + 0.137), styled);
  let r2 = select(random(2u), fract(k * 0.7548777 + 0.291), styled);
  let r3 = select(random(3u), fract(k * 0.5698403 + 0.733), styled);
  // By the weights of the views.
  let w = max(vec4f(params.front, params.side, params.far, params.close), vec4f(0.0));
  let x = r1 * max(w.x + w.y + w.z + w.w, 1e-3);
  var v = 0.0;
  if (x >= w.x) {
    v = 1.0;
  }
  if (x >= w.x + w.y) {
    v = 2.0;
  }
  if (x >= w.x + w.y + w.z) {
    v = 3.0;
  }
  state.view = v;
  let way = select(-1.0, 1.0, r2 < 0.5);
  if (v < 0.5) {
    state.cam.z = -params.distance;
    state.yaw = 0.015 * way;
    state.pitch = (r3 - 0.5) * 0.08;
    state.lens = 0.5;
    state.dir = way;
  } else if (v < 1.5) {
    state.cam.z = -params.distance * 0.7;
    state.yaw = way * params.side_angle * 0.0174533;
    state.pitch = (r3 - 0.5) * 0.14;
    state.lens = 0.5;
    state.dir = way;
  } else if (v < 2.5) {
    state.cam.z = -params.distance * 1.6;
    state.yaw = 0.0;
    state.pitch = (r3 - 0.5) * 0.05;
    state.lens = 0.42;
    state.dir = way;
  } else {
    state.cam.z = -params.distance * 0.85;
    state.yaw = way * 0.12;
    state.pitch = (r3 - 0.5) * 0.06;
    state.lens = 0.3;
    state.dir = way;
  }
  // Somewhere new on the wall (it has no end either way).
  state.cam.x += (40.0 + 200.0 * random(5u)) * select(-1.0, 1.0, random(6u) < 0.5);
  state.cam.y = 1.0 + 60.0 * random(7u);
  state.last_view = frame.time;
}

fn set_colours() {
  // The phosphor: the player's colour if it follows it, else its own. Either way held to one
  // lightness and a modest chroma, so every screen glows alike.
  var hue_of = params.tint.rgb;
  if (params.player_colour == 1u && frame.accent.a > 0.5) {
    hue_of = frame.accent.rgb;
  }
  let lab = rv_linear_to_oklab(hue_of);
  let h = atan2(lab.z, lab.y);
  let c = min(length(lab.yz), 0.17) * params.chroma;
  state.phosphor = max(rv_oklab_to_linear(vec3f(0.72, c * cos(h), c * sin(h))), vec3f(0.0));
  // The key light: a cool white.
  state.key = vec3f(0.82, 0.9, 1.0) * params.key;
}

// Where the scopes' time trace starts: the first rising zero crossing in the first half of the
// waveform, so the wave stands still; and a gain from its loudness so the trace keeps its size.
fn set_scope() {
  var sum = 0.0;
  var found = -1.0;
  var last = 0.0;
  for (var i = 0; i < 256; i++) {
    let x = f32(i) / 255.0;
    let s = rv_waveform(x);
    let m = 0.5 * (s.x + s.y);
    sum += dot(s, s);
    if (found < 0.0 && i > 0 && x < 0.5 && last < 0.0 && m >= 0.0) {
      found = x;
    }
    last = m;
  }
  state.trig = max(found, 0.0);
  let rms = sqrt(sum / 512.0);
  let want = 0.6 / max(rms * 2.5, 0.02);
  state.gain += (min(want, 40.0) - state.gain) * (1.0 - exp(-frame.dt / 0.3));
}

fn update() {
  let dt = frame.dt;
  let t = frame.time;
  if (frame.first == 1u) {
    state.cam = vec3f(0.0, 10.0, -params.distance);
    state.beat_len = 0.5;
    state.ens = 1.0;
    state.gain = 4.0;
    choose_view(-1.0);
    state.vel = params.cruise;
    state.vtgt = params.cruise;
  }

  let sure = music.bpm > 0.0 && music.beat_confidence > params.conf;
  let beat = music.beats > 0u && sure;
  state.sure = f32(sure);

  // The beat clock: it runs at the tempo and is set to the beat when one comes (never back).
  // Without a beat it runs at two a second.
  state.beat_len = select(0.5, 60.0 / max(music.bpm, 1.0), sure);
  state.clock += dt / state.beat_len;
  if (beat) {
    state.clock = max(ceil(state.clock - 0.35), state.clock);
  }

  // Energy: the section's with a chart, else the loudness; smoothed again for the speed.
  let chart = music.has_chart == 1u;
  if (chart) {
    state.energy = music.section_energy;
  } else {
    state.energy += (music.loudness - state.energy) * (1.0 - exp(-dt / 1.5));
  }
  state.ens += (0.5 + 1.5 * state.energy - state.ens) * (1.0 - exp(-dt / 0.6));
  let kin = max(f32(beat), params.hits * music.low * f32(sure));
  state.kick = max(state.kick * exp(-dt / max(params.kick_decay, 0.01)), kin);

  // Sections: the views change, the channels group anew, quiet ones leave sets dark.
  var fresh = false;
  if (chart && music.sections > 0u && t - state.last_view > 2.0) {
    state.section = music.section;
    choose_view(music.section_label);
    fresh = true;
  } else if (music.bars > 0u && t - state.last_view > params.view_gap) {
    let bar = u32(floor(music.bar));
    if (bar % max(u32(params.view_bars), 1u) == 0u && random(9u) * 100.0 < params.view_chance) {
      state.section += 101.0;
      choose_view(-1.0);
      fresh = true;
    }
  } else if (music.bpm <= 0.0 && t - state.last_view > 30.0) {
    state.section += 101.0;
    choose_view(-1.0);
    fresh = true;
  }
  // Some sets are always off, and more in a quiet section.
  state.dark = params.off_always + select(0.0, params.off * pow(max(1.0 - state.energy, 0.0), 1.5), chart);

  // How fast the music sounds (as in Neon Street): the song's own pace and, stretched further,
  // this moment's against it.
  let song = select(5.0, music.song_feel, music.song_feel > 0.0);
  let here = select(song, music.feel, music.feel > 0.0);
  if (state.feel <= 0.0) {
    state.feel = here;
  }
  state.feel += (here - state.feel) * (1.0 - exp(-dt / params.feel_smooth));
  let pace = clamp(pow(song / 5.0, params.song_pace) * pow(state.feel / song, params.moment_pace), 0.25, 3.0);

  // The camera glides along the wall at a cruise, pushed on each beat and easing back; from the
  // side faster, close up slower.
  let cruise = params.cruise * pace * (1.0 + params.energy_speed * (state.ens - 1.0));
  state.vtgt = cruise + (state.vtgt - cruise) * exp(-dt / max(params.ease, 0.05)) + f32(beat) * params.push * cruise;
  if (fresh) {
    state.vel = state.vtgt;
  }
  state.vel += (state.vtgt - state.vel) * (1.0 - exp(-dt / max(params.accel, 0.01)));
  let view_speed = select(select(select(1.0, 0.45, state.view > 2.5), 1.6, state.view > 0.5 && state.view < 1.5), 1.0, state.view < 0.5);
  state.cam.x += dt * state.dir * state.vel * view_speed;

  set_colours();
  set_scope();

  post.exposure = params.exposure;
  post.bloom_intensity = params.glow;
  post.grain = params.grain;
  post.chromatic = 0.0;
}
