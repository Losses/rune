// Once per frame: the spirits fly down the line of torii, a gate on every beat, drawing in to
// the middle of each gate as they pass it and spreading out between; they surge on the beat, each
// follows a band of the music, a snare flicks them aside, an accent sends them leaping into the
// sky. The camera watches them from one of five places, cut to at a section and glided to
// between. The light changes with the music and the waves follow its loudness.

fn random(n: u32) -> f32 {
  return rv_unit(frame.seed * 2654435761u + n);
}

// Pick the view: 0 chasing through the gates, 1 low beside, 2 above, 3 ahead looking back,
// 4 high and wide. A section cuts there (by its label, so every chorus is seen the same way);
// otherwise the camera glides.
fn choose_view(label: f32) {
  let styled = label >= 0.0 && params.same_view == 1u;
  // Two shots, cutting between them: 1 high on the left and a little behind, looking down on the
  // spirits; 2 from behind and to one side, lower, looking on past them to the light far ahead.
  var v = 1.0 + floor(select(random(1u), fract((label + 1.0) * 0.6180339 + 0.137), styled) * 2.0);
  if (!styled && v == state.view) {
    v = 3.0 - v;
  }
  state.view = v;
  state.lat = select(-1.0, 1.0, select(random(3u), fract((label + 1.0) * 0.7548777), styled) < 0.5);
  state.last_view = frame.time;
}

// Pick how the spirits move: 0 braid, 1 snake, 2 fan, 3 weaving in and out of the gates (twice
// as likely as the others); and how many fly.
fn choose_mode(label: f32) {
  var m: f32;
  if (params.mode > 0u) {
    m = f32(params.mode - 1u);
  } else {
    let r = select(random(31u), fract(label * 0.5698403 + 0.31), label >= 0.0);
    m = min(floor(r * 5.0), 3.0);
  }
  state.mode = m;
  let r = select(random(32u), fract(label * 0.381966 + 0.7), label >= 0.0);
  state.count = mix(params.min_spirits, f32(SPIRITS), r);
  state.last_mode = frame.time;
}

// Pick the light, at a cut: 0 day, 1 a low sun ahead, 2 blue hour, 3 night; by the section's
// label, so the same kind of section has the same light.
fn choose_light(label: f32) {
  var m: f32;
  if (params.light_mode > 0u) {
    m = f32(params.light_mode - 1u);
  } else if (label >= 0.0) {
    m = floor(fract(label * 0.381966 + 0.21) * 4.0);
  } else {
    m = f32((u32(state.light_to) + 1u + u32(random(11u) * 3.0)) % 4u);
  }
  state.light_next = m;
  state.light_from = m;
  state.light_to = m;
  state.light_mix = 1.0;
  state.last_light = frame.time;
}

// Where the line runs at z, at the gates' opening height: the spirits' track.
fn track(z: f32) -> vec3f {
  return vec3f(way_x(z), params.through, z);
}

fn update() {
  let dt = min(frame.dt, 0.05);
  let t = frame.time;
  if (frame.first == 1u) {
    state.s = random(4u) * 3000.0;
    state.vel = params.cruise;
    state.vtarget = params.cruise;
    state.tempo = 1.0;
    state.ens = 1.0;
    state.lens = 1.0;
    state.wave = params.wave_height;
    choose_view(-1.0);
    choose_mode(-1.0);
    choose_light(-1.0);
    state.light_from = state.light_next;
    state.light_to = state.light_next;
    state.light_mix = 1.0;
  }

  let sure = music.bpm > 0.0 && music.beat_confidence > params.conf;
  let beat = f32(music.beats > 0u && sure);

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

  // The kick (the sea heaves, the water rings round the pillars) and the beat's surge.
  let kin = max(beat, params.hits * music.low * f32(sure));
  state.kick = max(state.kick * exp(-dt / params.kick_decay), kin);
  if (kin > 0.6 && state.since_kick > 0.2) {
    state.since_kick = 0.0;
  }
  state.since_kick += dt;
  if (beat > 0.5 || (!sure && kin > 0.6)) {
    state.surge = 1.0;
  }
  state.surge *= exp(-dt / max(params.surge_decay, 0.02));
  // The live levels, quick up and a little slower down; the snare (or a clap) flicks.
  let now_levels = vec3f(music.bass, music.mids, music.treble);
  let up_k = 1.0 - exp(-dt / 0.03);
  let down_k = 1.0 - exp(-dt / 0.15);
  state.levels = select(state.levels + (now_levels - state.levels) * down_k, state.levels + (now_levels - state.levels) * up_k, now_levels > state.levels);
  let snare = select(music.mids > 1.6, music.mid > 0.7, chart);
  if (snare && state.flick < 0.3) {
    state.flick = 1.0;
  }
  state.flick *= exp(-dt / 0.18);

  // The waves stand as tall as the music is loud.
  let wave_to = params.wave_height * (0.35 + 1.3 * params.wave_energy * state.energy);
  state.wave += (wave_to - state.wave) * (1.0 - exp(-dt / 0.5));
  // Accents: a big swell breaks round the pillars, and the spirits leap into the sky.
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
    state.swell = max(state.swell, accent * params.swell);
    if (state.leap <= 0.0 || state.leap >= 1.0) {
      state.leap = 0.001;
      state.leap_power = accent;
    }
  }
  // The sea holds still, and every few beats (and on an accent) a new set of triangles takes its
  // place, as tall as the music is loud then, taller on the kick and in an accent's swell.
  // Without a beat it changes every second or so.
  let beat_no = u32(max(floor(music.beat), 0.0));
  let sea_turn = (beat > 0.5 && beat_no % max(u32(params.sea_beats), 1u) == 0u) || accent > 0.0 || (!sure && t - state.sea_last > 1.2);
  if (sea_turn || frame.first == 1u) {
    state.sea_gen = f32((u32(state.sea_gen) + 1u) % 4096u);
    state.sea_amp = state.wave * (1.0 + params.wave_kick * kin) + 1.2 * state.swell;
    state.sea_last = t;
  }
  state.swell *= exp(-dt / 3.0);
  state.swell_phase = fract(state.swell_phase - dt * 0.12);
  if (state.leap > 0.0 && state.leap < 1.0) {
    state.leap = min(state.leap + dt / max(params.leap_time, 0.2), 1.0);
  }

  // Shots, patterns and light: at a section's start by its label, the camera cutting and the
  // light changing with it; otherwise every few bars the shot cuts (and every other time the
  // light with it) and the pattern changes.
  if (chart && music.sections > 0u && t - state.last_view > 10.0) {
    choose_view(music.section_label);
    choose_mode(music.section_label);
    choose_light(music.section_label);
  } else if (music.bars > 0u) {
    let bar = u32(floor(music.bar));
    if (t - state.last_view > 12.0 && bar % max(u32(params.view_bars), 1u) == 0u) {
      choose_view(-1.0);
      if (!chart && random(12u) < 0.5) {
        choose_light(-1.0);
      }
    }
    if (t - state.last_mode > 4.0 && (bar + 2u) % max(u32(params.mode_bars), 1u) == 0u) {
      choose_mode(-1.0);
    }
  } else if (music.bpm <= 0.0) {
    if (t - state.last_view > 12.0) {
      choose_view(-1.0);
      if (random(12u) < 0.5) {
        choose_light(-1.0);
      }
    }
    if (t - state.last_mode > 16.0) {
      choose_mode(-1.0);
    }
  }

  // Gliding as fast as the music sounds (as in Neon Street), locked to the beat: a gate passes
  // on every beat (or every two or four, or two a beat, whichever is nearest that pace).
  let song = select(5.0, music.song_feel, music.song_feel > 0.0);
  let here = select(song, music.feel, music.feel > 0.0);
  if (state.feel <= 0.0) {
    state.feel = here;
  }
  state.feel += (here - state.feel) * (1.0 - exp(-dt / params.feel_smooth));
  let pace = pow(song / 5.0, params.song_pace) * pow(state.feel / song, params.moment_pace);
  state.tempo += (clamp(pace, 0.25, 3.0) - state.tempo) * (1.0 - exp(-dt / 1.0));
  let cruise = params.cruise * state.tempo * (1.0 + params.energy_speed * (state.ens - 1.0));
  state.vtarget = cruise + (state.vtarget - cruise) * exp(-dt / 0.6);
  var want_vel = state.vtarget;
  if (sure && params.beat_lock > 0.0) {
    let one = params.spacing * music.bpm / 60.0;
    let want = clamp(round(log2(max(state.vtarget, 0.1) / one)), -2.0, 1.0);
    if (state.per_beat <= 0.0 || (music.bars > 0u && abs(want - log2(state.per_beat)) > 0.5)) {
      state.per_beat = exp2(want);
    }
    var err = fract(music.beat * state.per_beat) - fract(state.s / params.spacing);
    err -= round(err);
    want_vel = one * state.per_beat + err * params.spacing / 0.7;
    state.lock = min(state.lock + dt / 1.5, 1.0);
  } else {
    state.lock = max(state.lock - dt / 1.5, 0.0);
  }
  let goal = mix(state.vtarget, want_vel, state.lock * params.beat_lock);
  state.vel += (goal - state.vel) * (1.0 - exp(-dt / mix(0.5, 0.2, state.lock)));
  state.s += dt * max(state.vel, 0.0);

  // The spirits. Their centre runs at s, through the middle of each gate as the beat comes.
  let s = state.s;
  let n = clamp(round(state.count + (state.energy - 0.5) * 2.0), 1.0, f32(SPIRITS));
  // How hard they move, by how loud it is this moment (not the section's average): the chart's
  // loudness curve where there is a chart (worked out ahead, so it does not lag), else the live
  // loudness of the last second against the last twenty; smoothed over 0.8 s, which follows the
  // phrases but not every hit (the chart's curve swings with each drum). Quiet, small calm moves;
  // loud, wide and sharp ones. It scales how far they go and how stiffly
  // they spring, never where they are in their figures, so a change of loudness does not make
  // them jump.
  let moment_now = select(clamp((music.energy - 0.6) / 1.0, 0.0, 1.0), music.loudness, chart);
  state.moment += (moment_now - state.moment) * (1.0 - exp(-dt / 0.8));
  let drive = mix(1.0, mix(0.35, 1.6, clamp(state.moment, 0.0, 1.0)), params.energy_motion);
  let k = params.stiff * mix(0.7, 1.3, clamp((drive - 0.35) / 1.25, 0.0, 1.0));
  let c = 2.0 * sqrt(k) * params.damp;
  // Through a gate they draw in to its middle; between gates they spread.
  let between = sin(3.14159265 * fract(gate_pos(s)));
  let open = mix(params.pinch, 1.0, between * between);
  // A leap: up into the sky and back down, in an arc.
  let leap_h = select(0.0, sin(3.14159265 * state.leap) * params.leap * state.leap_power, state.leap > 0.0 && state.leap < 1.0);
  // Everything the spirits do is counted in beats: each beat a step, quick off the mark and
  // settled before the next (without a beat, two a second).
  let bp = select(t * 2.0 * state.tempo, music.beat, sure);
  let fb = floor(bp);
  let snap = 1.0 - pow(1.0 - fract(bp), 4.0);
  let stepped = fb + snap;
  let per_bar = max(select(4.0, music.beats_per_bar, music.beats_per_bar > 0.5), 1.0);
  let downbeat = fb - per_bar * floor(fb / per_bar) < 0.5;
  for (var h = 0u; h < SPIRITS; h++) {
    let fh = f32(h);
    let on = f32(fh < n);
    var p = state.spirit[h].xyz;
    var v = state.spirit_vel[h].xyz;
    var w = state.spirit[h].w;
    if (on > 0.5 && w < 0.02) {
      // A spirit coming back starts on the track, its path gathered to it.
      p = track(s - 2.0);
      v = vec3f(0.0, 0.0, state.vel);
      for (var j = 0u; j < POINTS; j++) {
        state.path[h * POINTS + j] = vec4f(p, 0.0);
      }
    }
    w += (on - w) * (1.0 - exp(-dt / 0.4));

    var z = s;
    var ang = fh / max(n, 1.0) * 6.2831853;
    var rad = params.spread * open * (0.7 + 0.3 * drive);
    if (state.mode < 0.5) {
      // A braid: abreast, whirling round the track.
      ang += stepped * params.orbit * 1.2;
    } else if (state.mode < 1.5) {
      // A snake: one behind the other, weaving.
      z = s - fh * params.stagger * 2.0;
      let b2 = sin(3.14159265 * fract(gate_pos(z)));
      ang = stepped * params.orbit * 0.8 + fh * 0.9;
      rad = params.spread * mix(params.pinch, 1.0, b2 * b2) * 0.8;
    } else if (state.mode < 2.5) {
      // A fan: each its own way round, opening wide between the gates, slowly turning.
      ang += stepped * params.orbit * 0.5;
      rad *= 1.3;
      z = s - fh * params.stagger * 0.5;
    }
    // Now and then, on a beat, a spirit strays from the others for a few beats: over the top of a
    // gate, out wide round the line, skimming the water, or dashing ahead; then it comes back.
    var trip = state.trip[h];
    if (beat > 0.5 && trip.w <= 0.0 && random(40u + h) < params.trip_chance * drive) {
      let r = random(50u + h);
      let side = select(-1.0, 1.0, random(60u + h) < 0.5);
      let ahead_gate = gate(floor(gate_pos(s)) + 1.0);
      var off = vec3f(0.0, gate_top(ahead_gate) + 1.8 - params.through, 2.0);
      if (r > 0.75) {
        off = vec3f(0.0, 1.0, 9.0 + 4.0 * random(70u + h));
      } else if (r > 0.5) {
        off = vec3f(side * 2.5, 0.9 - params.through, 3.0);
      } else if (r > 0.25) {
        off = vec3f(side * (5.0 + 4.0 * random(70u + h)), 1.0 + 2.0 * random(80u + h), 0.0);
      }
      trip = vec4f(off, 0.001);
      let beat_s = select(0.5, 60.0 / max(music.bpm, 1.0), sure);
      state.trip_len[h] = vec4f(beat_s * mix(1.5, 3.0, random(90u + h)), 0.0, 0.0, 0.0);
    }
    var away = 0.0;
    if (trip.w > 0.0) {
      trip.w += dt / max(state.trip_len[h].x, 0.1);
      if (trip.w >= 1.0) {
        trip.w = 0.0;
      }
      away = sin(3.14159265 * trip.w);
    }
    state.trip[h] = trip;
    // Off on its own it does not draw in at the gates; the formation opens as the music swells.
    rad = mix(rad, params.spread, away);
    let swell_k = clamp(0.4 + 0.7 * (state.levels.x + state.levels.y) * 0.5, 0.4, 1.8);
    rad *= mix(1.0, swell_k, params.breathe);
    // Every few beats each spirit moves to a new spot of its own (further on a bar's first beat),
    // and a kick throws them all out from the middle for a moment.
    // Only every few beats (pose_beats), so they do not fidget.
    let every = max(params.pose_beats, 1.0);
    let pb = floor(fb / every);
    let into = (fb - pb * every + snap) / every;
    let pose_snap = 1.0 - pow(1.0 - clamp(into * every, 0.0, 1.0), 3.0);
    let big = (pb * every) - per_bar * floor(pb * every / per_bar) < 0.5;
    let pose_now = (vec3f(hash(vec2f(fh, pb)), hash(vec2f(fh + 7.0, pb)), hash(vec2f(fh + 13.0, pb))) * 2.0 - 1.0) * select(1.0, 1.6, big);
    let pose_was = (vec3f(hash(vec2f(fh, pb - 1.0)), hash(vec2f(fh + 7.0, pb - 1.0)), hash(vec2f(fh + 13.0, pb - 1.0))) * 2.0 - 1.0);
    let wander = mix(pose_was, pose_now, pose_snap) * vec3f(1.0, 0.7, 1.5) * params.wander * 1.2 * drive;
    rad *= 1.0 + params.kick_burst * state.kick * drive;
    let base = track(z);
    var place = base + vec3f(cos(ang), sin(ang) * 0.8, 0.0) * rad + wander * (1.0 - 0.6 * (1.0 - open) * (1.0 - away)) + trip.xyz * away;
    if (state.mode > 2.5) {
      // Weaving: through the middle of every gate, and between the gates swinging out round its
      // pillars or up over its top, each spirit its own way and reach.
      let zz = s - fh * params.stagger * 0.7;
      let g = gate_pos(zz);
      let swing_x = sin(3.14159265 * g + fh * 1.9) * sin(3.14159265 * g);
      let swing_y = sin(3.14159265 * g) * sin(3.14159265 * g * 0.5 + fh * 2.7);
      let reach_x = params.weave * mix(3.0, 6.0, hash(vec2f(fh, 1.7))) * select(-1.0, 1.0, hash(vec2f(fh, 2.9)) < 0.5) * (0.6 + 0.4 * drive);
      let reach_y = params.weave * mix(0.5, 4.0, hash(vec2f(fh, 3.3))) * (0.6 + 0.4 * drive);
      place = track(zz) + vec3f(swing_x * reach_x, abs(swing_y) * reach_y, 0.0) + wander * 0.3 * (1.0 - away) + trip.xyz * away;
    }
    // A snare flicks each its own way; the surge throws them ahead; a leap lifts them.
    let fa = hash(vec2f(fh, floor(t * 4.0))) * 6.2831853;
    place += vec3f(cos(fa), sin(fa), 0.0) * state.flick * params.flick * drive;
    place.z += state.surge * params.surge * drive;
    place.y += leap_h * (1.0 + 0.15 * fh);
    let a = (place - p) * k - (v - vec3f(0.0, 0.0, state.vel)) * c;
    v += a * dt;
    p += v * dt;
    // Never below the waves.
    p.y = max(p.y, 0.6 + state.wave);
    state.spirit[h] = vec4f(p, w);
    state.spirit_vel[h] = vec4f(v, 0.0);
  }

  // The paths: a point written every so often into the ring.
  state.until -= dt;
  if (state.until <= 0.0) {
    state.until = max(state.until + params.trail_time / f32(POINTS - 1u), 0.0);
    state.slot = (state.slot + 1u) % POINTS;
    state.written += 1.0;
    for (var h = 0u; h < SPIRITS; h++) {
      state.path[h * POINTS + state.slot] = state.spirit[h];
    }
  }

  // The camera, by its shot, moving on with the spirits; it never sways or rolls, nor rises with
  // their leaps.
  var cam: vec3f;
  var look: vec3f;
  var lens = 1.0;
  if (state.view > 1.5) {
    // From behind and to one side, a little above the water and looking slightly down (lower,
    // the sea grazes the view and shows its triangles' faults), on past the spirits to the light
    // far ahead; a slightly long lens.
    let cz = s - 18.0;
    // Out between the line's gates and the strays, clear of both.
    cam = vec3f(path_x(cz) + state.lat * 9.0, params.rear_height, cz);
    look = vec3f(path_x(s + 30.0), 1.0, s + 30.0);
    lens = 0.75;
  } else {
    // High up on the left and well behind the spirits, looking on ahead down the line to the
    // horizon rather than down on them, a wide lens (left is -x, the way the camera looks being
    // +z).
    let cz = s - params.cam_dist * 3.0;
    cam = vec3f(path_x(cz) - params.side_offset * 1.6, params.high, cz);
    look = vec3f(path_x(s + 40.0), 0.0, s + 40.0);
    lens = 1.05;
  }
  state.cam_pos = cam;
  state.cam_fwd = normalize(look - cam);
  state.roll = 0.0;
  state.lens = lens;
  let right = normalize(cross(vec3f(0.0, 1.0, 0.0), state.cam_fwd));
  state.cam_right = right;
  state.cam_up = cross(state.cam_fwd, right);

  // Each spirit's ribbon and flakes, made in the world and projected for the passes.
  for (var h = 0u; h < SPIRITS; h++) {
    build_ribbon(h);
  }
  list_torii();

  post.bloom_intensity = params.glow * (0.6 + 0.5 * state.energy);
  post.exposure = params.exposure;
  // The theme's hue.
  let follow = params.player_colour == 1u && frame.accent.a > 0.5;
  let theme = oklab(select(params.theme.rgb, frame.accent.rgb, follow));
  state.theme_hue = atan2(theme.z, theme.y);
  // The sea in marks fringes a little red and blue, as on a screen.
  post.chromatic = select(0.0, 0.0022, params.sea_look == 0u);
}

// ---- The torii in sight ----
// Once a frame, nearest first: the slots out from the camera's, both ways as far as it looks
// along the line (up to 160), each with its torii of the line and (within 400 m) its two strays.
// Each one in sight is worked out once and kept for the passes, and listed in every tile of the
// screen its box covers (the box its ray test starts with, turned and leaned as it stands, its
// eight corners put on the screen; its bounding sphere when a corner is behind the camera). One
// small on the screen is marked to be drawn plainly.
fn list_torii() {
  for (var k = 0u; k < TILES; k++) {
    state.tile_n[k] = 0u;
  }
  state.torii_n = 0u;
  let sp = params.spacing;
  let here = floor(state.cam_pos.z / sp);
  let ahead = 3.0 + 157.0 * smoothstep(-0.2, 0.6, state.cam_fwd.z);
  let behind = 3.0 + 157.0 * smoothstep(0.2, -0.6, state.cam_fwd.z);
  for (var k = 0.0; k <= max(ahead, behind); k += 1.0) {
    for (var way = 1.0; way >= -1.0; way -= 2.0) {
      if ((way < 0.0 && (k < 0.5 || k > behind)) || (way > 0.0 && k > ahead)) {
        continue;
      }
      let i = here + k * way;
      see_torii(gate(i), i);
      if (k * sp < 400.0) {
        for (var sd = 0.0; sd < 2.0; sd += 1.0) {
          let j = 2.0 * i + sd;
          if (stray_place(j).w > 0.5) {
            see_torii(stray(j), -1.0 - j);
          }
        }
      }
    }
  }
}

fn see_torii(g0: Gate, slot: f32) {
  let n = state.torii_n;
  if (g0.here < 0.5 || n >= TORII) {
    return;
  }
  var g = g0;
  let centre = vec3f(g.x, g.base + 3.0 * g.size, g.z);
  let r = 5.0 * g.size;
  let v = centre - state.cam_pos;
  let zc = dot(v, state.cam_fwd);
  if (zc < -r) {
    return;
  }
  let focal = 1.0 / (state.lens * 0.8);
  // Its box, as gate_hit has it in the torii's own frame (in sizes, from its foot): a ruin's
  // reaches out to the pieces lying beside it.
  let span = select(vec3f(3.6, 3.4, 0.5), vec3f(4.4, 3.4, 3.4), g.broken > 1.5);
  let ya = atan(path_dx(g.z)) + g.turn;
  let la = g.lean;
  var lo_s = vec2f(1e9);
  var hi_s = vec2f(-1e9);
  var whole = false;
  for (var k = 0u; k < 8u; k++) {
    let corner = vec3f(0.0, 3.0, 0.0) + span * (vec3f(f32(k & 1u), f32((k >> 1u) & 1u), f32((k >> 2u) & 1u)) * 2.0 - 1.0);
    // Out of the torii's frame: unlean, unturn, to its size and place.
    let u = vec3f(corner.x * cos(la) + corner.y * sin(la), -corner.x * sin(la) + corner.y * cos(la), corner.z);
    let w = vec3f(u.x * cos(ya) + u.z * sin(ya), u.y, -u.x * sin(ya) + u.z * cos(ya)) * g.size + vec3f(g.x, g.base, g.z);
    let q = w - state.cam_pos;
    let qz = dot(q, state.cam_fwd);
    if (qz < 0.3) {
      whole = true;
      break;
    }
    let s = focal * vec2f(dot(q, state.cam_right), dot(q, state.cam_up)) / qz;
    lo_s = min(lo_s, s);
    hi_s = max(hi_s, s);
  }
  if (whole) {
    // A corner behind the camera: the bounding sphere's circle instead, or the whole screen.
    var c = vec2f(0.0);
    var rad = 10.0;
    if (zc > r * 1.05) {
      c = focal * vec2f(dot(v, state.cam_right), dot(v, state.cam_up)) / zc;
      rad = focal * r / sqrt(zc * zc - r * r);
    }
    lo_s = c - rad;
    hi_s = c + rad;
  }
  let aspect = frame.aspect;
  if (lo_s.x > aspect * 0.5 || hi_s.x < -aspect * 0.5 || lo_s.y > 0.5 || hi_s.y < -0.5) {
    return;
  }
  // Small on the screen (under about a twelfth of its height across), drawn plainly.
  g.lod = f32(max(hi_s.x - lo_s.x, hi_s.y - lo_s.y) < 0.08);
  state.torii_a[n] = vec4f(g.kind, g.broken, g.size, g.x);
  state.torii_b[n] = vec4f(g.z, g.base, g.lean, max(length(v) - r, 0.0));
  state.torii_c[n] = vec4f(g.turn, g.askew, g.lod, slot);
  state.torii_n = n + 1u;
  // The tiles it covers (uv: 0..1, y down).
  let lo = vec2f(lo_s.x / aspect + 0.5, 0.5 - hi_s.y);
  let hi = vec2f(hi_s.x / aspect + 0.5, 0.5 - lo_s.y);
  let x0 = u32(clamp(floor(lo.x * f32(TX)), 0.0, f32(TX - 1u)));
  let x1 = u32(clamp(floor(hi.x * f32(TX)), 0.0, f32(TX - 1u)));
  let y0 = u32(clamp(floor(lo.y * f32(TY)), 0.0, f32(TY - 1u)));
  let y1 = u32(clamp(floor(hi.y * f32(TY)), 0.0, f32(TY - 1u)));
  for (var ty = y0; ty <= y1; ty++) {
    for (var tx = x0; tx <= x1; tx++) {
      let k = ty * TX + tx;
      let m = state.tile_n[k];
      if (m < LIST) {
        state.tile[k * LIST + m] = n;
        state.tile_n[k] = m + 1u;
      }
    }
  }
}

// The point of spirit h's path j back (0 the spirit itself), newest first.
fn path_point(h: u32, j: u32) -> vec3f {
  if (j == 0u) {
    return state.spirit[h].xyz;
  }
  let k = min(j, POINTS - 1u);
  return state.path[h * POINTS + (state.slot + POINTS - (k - 1u)) % POINTS].xyz;
}

// Spirit h's paper ribbon: at every point of its path a crosswise edge, turned about the path
// by a twist that stays with the point, its middle folded to one side and the other in turn (as
// the zigzag of shide paper), narrow at the head, widest just behind it and tapering to the end;
// wider on the beat and with its band. Two flat triangles join each point to the next. Then the
// flakes, shed one at most from each point, falling and tumbling down to the water.
fn build_ribbon(h: u32) {
  let present = state.spirit[h].w > 0.01;
  let interval = params.trail_time / f32(POINTS - 1u);
  // How far the head is towards the next point to be written.
  let frac = clamp(1.0 - state.until / max(interval, 1e-3), 0.0, 1.0);
  let live = mix(1.0, pow(clamp(0.2 + 0.8 * state.levels[h % 3u], 0.2, 2.1), 1.5), params.react);
  var e1: array<vec3f, 40>;
  var e2: array<vec3f, 40>;
  var lo = vec2f(1e9);
  var hi = vec2f(-1e9);
  // The box of each stretch: its own points, the next one (its last triangles reach it) and its
  // flakes.
  var clo: array<vec2f, 8>;
  var chi: array<vec2f, 8>;
  for (var k = 0u; k < 8u; k++) {
    clo[k] = vec2f(1e9);
    chi[k] = vec2f(-1e9);
  }
  for (var j = 0u; j < POINTS; j++) {
    let p = path_point(h, j);
    let ahead = path_point(h, max(j, 1u) - 1u);
    let behind = path_point(h, min(j + 1u, POINTS - 1u));
    var tng = ahead - behind;
    if (dot(tng, tng) < 1e-8) {
      tng = vec3f(0.0, 0.0, 1.0);
    }
    tng = normalize(tng);
    var side = cross(tng, vec3f(0.0, 1.0, 0.0));
    if (dot(side, side) < 1e-6) {
      side = vec3f(1.0, 0.0, 0.0);
    }
    side = normalize(side);
    let up = cross(side, tng);
    // Which point this is, counted from the start, so its twist and fold move with it.
    let serial = select(state.written - f32(j) + 1.0, state.written + frac, j == 0u);
    let turn = serial * params.twist + f32(h) * 1.9;
    let across = side * cos(turn) + up * sin(turn);
    let age = clamp((f32(j) - 1.0 + frac) / f32(POINTS - 1u), 0.0, 1.0);
    let w = params.trail_width * 0.5 * mix(0.35, 1.0, smoothstep(0.0, 0.12, age)) * pow(1.0 - age, 0.6) * (0.75 + 0.25 * live) * (1.0 + 0.6 * state.surge * (1.0 - age));
    let fold = params.zigzag * w * select(-0.6, 0.6, fract(serial * 0.5) < 0.5);
    e1[j] = p + across * (fold + w);
    e2[j] = p + across * (fold - w);
    let q1 = project(e1[j]);
    let q2 = project(e2[j]);
    let front = present && q1.z > 0.3 && q2.z > 0.3;
    state.ribbon[h * POINTS + j] = vec4f(q1.xy, q2.xy);
    state.ribbon_z[h * POINTS + j] = vec4f(q1.z, q2.z, f32(front), age);
    if (front) {
      let a = min(q1.xy, q2.xy);
      let b = max(q1.xy, q2.xy);
      lo = min(lo, a);
      hi = max(hi, b);
      let k = min(j / CHUNK, 7u);
      clo[k] = min(clo[k], a);
      chi[k] = max(chi[k], b);
      // The point that ends the stretch before belongs to it too.
      if (j % CHUNK == 0u && j > 0u) {
        clo[k - 1u] = min(clo[k - 1u], a);
        chi[k - 1u] = max(chi[k - 1u], b);
      }
    }

    // The flake shed from this point, if it sheds one: from somewhere across the ribbon,
    // falling at its own pace, swaying, tumbling, floating once it reaches the water and fading
    // before the point is written over.
    var size = 0.0;
    let r = hash(vec2f(serial * 0.731, f32(h) * 3.1));
    if (j >= 2u && r < params.glitter_amount && present) {
      let t_age = age * params.trail_time;
      let r2 = hash(vec2f(serial * 1.37, f32(h) + 7.0));
      let r3 = hash(vec2f(serial * 2.11, f32(h) + 13.0));
      var q = p + across * (fold + w * (2.0 * r2 - 1.0));
      let fall = params.flake_fall * (0.6 + 0.8 * r3);
      q.y = max(q.y - fall * t_age, 0.15 + 0.4 * state.wave);
      q += vec3f(sin(t_age * 2.3 + r * 6.3), 0.0, cos(t_age * 1.7 + r2 * 6.3)) * 0.25 * min(t_age, 1.0);
      let a = t_age * (2.0 + 4.0 * r2) + r * 6.3;
      let b = t_age * (1.0 + 3.0 * r3) + r2 * 6.3;
      let n = vec3f(sin(a) * cos(b), cos(a), sin(a) * sin(b));
      let sq = project(q);
      if (sq.z > 0.3) {
        size = params.glitter_size / (sq.z * state.lens * 0.8) * (1.0 - smoothstep(0.75, 1.0, age)) * smoothstep(0.0, 0.05, age);
        state.flake[h * POINTS + j] = vec4f(sq.xy, sq.z, t_age * (1.5 + 3.0 * r3));
        state.flake_n[h * POINTS + j] = vec4f(n, size);
        lo = min(lo, sq.xy - size);
        hi = max(hi, sq.xy + size);
        let k = min(j / CHUNK, 7u);
        clo[k] = min(clo[k], sq.xy - size);
        chi[k] = max(chi[k], sq.xy + size);
      }
    }
    if (size <= 0.0) {
      state.flake_n[h * POINTS + j] = vec4f(0.0, 1.0, 0.0, 0.0);
    }
  }
  // The two triangles from each point to the next: (e1 here, e2 here, e1 next) and
  // (e2 here, e2 next, e1 next).
  for (var j = 0u; j + 1u < POINTS; j++) {
    let n1 = cross(e2[j] - e1[j], e1[j + 1u] - e1[j]);
    let n2 = cross(e2[j + 1u] - e2[j], e1[j + 1u] - e2[j]);
    state.ribbon_n[2u * (h * POINTS + j)] = vec4f(normalize(n1 + vec3f(0.0, 1e-6, 0.0)), 0.0);
    state.ribbon_n[2u * (h * POINTS + j) + 1u] = vec4f(normalize(n2 + vec3f(0.0, 1e-6, 0.0)), 0.0);
  }
  state.bounds[h] = vec4f(lo - 0.01, hi + 0.01);
  for (var k = 0u; k < 8u; k++) {
    state.chunks[h * 8u + k] = vec4f(clo[k] - 0.01, chi[k] + 0.01);
  }
}
