// Once per frame: the spirits fly down the line of torii, a gate on every beat, drawing in to
// the middle of each gate as they pass it and spreading out between; they surge on the beat, each
// follows a band of the music, a snare flicks them aside, an accent sends them leaping into the
// sky. The camera watches them from one of four shots, cut between by the runtime's director. The
// light changes with the music and the waves follow its energy.

fn random(n: u32) -> f32 {
  return rv_unit(frame.seed * 2654435761u + n);
}

// Take up the director's shot: its index is the view (0 high behind, looking down the line; 1
// low behind to one side, looking on to the light; 2 close beside the spirits, flying with them;
// 3 ahead of them to one side, watching them come on; then the burst's four: 4 above and behind to one side,
// 5 close beside their heads, 6 a long lens on their heads from ahead, 7 high above the line), and
// its seed picks the side of the line. The burst's cuts change side each beat. No shot glides, so
// all are simply cut to.
fn take_shot() {
  state.view = f32(shot.index);
  state.lat = select(-1.0, 1.0, shot.seed < 0.5);
  if (shot.fill > 0u) {
    state.lat = select(-1.0, 1.0, shot.fill % 2u == 1u);
  }
  state.cuts_seen = f32(shot.cuts);
}

// Set how the spirits move: 0 braid, 1 snake, 2 fan, 3 weaving in and out of the gates. `draw` is
// one of five, the last two both weaving, so that it is twice as likely as the others; `crowd`
// (0 to 1) says how many fly.
fn choose_mode(draw: f32, crowd: f32) {
  var m = min(draw, 3.0);
  if (params.mode > 0u) {
    m = f32(params.mode - 1u);
  }
  state.mode = m;
  state.count = mix(params.min_spirits, f32(SPIRITS), crowd);
  state.last_mode = frame.time;
}

// A pick of the music's for the pattern: the draw of a kind of section, and how many fly with it.
fn choose_mode_of(draw: f32) {
  choose_mode(draw, fract(draw * 0.618034 + 0.7));
}

// Where the line runs at z, at the gates' opening height: the spirits' track.
fn track(z: f32) -> vec3f {
  return vec3f(way_x(z), params.through, z);
}

fn update() {
  let dt = rv_dt();
  let t = frame.time;
  if (frame.first == 1u) {
    state.s = random(4u) * 3000.0;
    state.vel = params.cruise;
    state.vtarget = params.cruise;
    state.lens = 1.0;
    state.wave = params.wave_height;
    take_shot();
    // The first pattern is drawn from the scene's seed. The first light is the runtime's (a look
    // that opens: never day). The first kind of section the music says (`pattern`, -1 while it
    // says none) is seen as a change.
    choose_mode(floor(random(31u) * 5.0), random(32u));
    state.pattern_seen = -2.0;
  }

  // Without a beat (the runtime tells of none) the hits of the music stand in for it.
  let beat = input.beat;
  let onset = input.beat > 0.5 && input.tick < 0.5;
  // How much the beat is followed right now (0 to 1), from the runtime: it fades what is locked to
  // the beat in and out.
  let locked = input.locked;

  // The kick (the sea heaves, the water rings round the pillars) and the beat's surge.
  let kin = max(beat, input.kick);
  state.kick = max(state.kick * exp(-dt / params.kick_decay), kin);
  // The beat's surge: its envelope, with the decay time of `surge_decay`, from the runtime.
  state.surge = input.surge_env;
  // The snare (or a clap) flicks, once for a hit until it has mostly died away.
  if (input.snare_hit > 0.5 && state.flick < 0.3) {
    state.flick = 1.0;
  }
  state.flick *= exp(-dt / 0.18);

  // The waves stand as tall as the music is loud.
  // (`wave` is the loudness, already swelling slowly.)
  state.wave = params.wave_height * (0.35 + 1.3 * params.wave_energy * input.wave);
  // Accents: a big swell breaks round the pillars, and the spirits leap into the sky. A cue of
  // strength 1 (a section that lifts the energy) is the strongest, a hard hit inside a section
  // (an impact) a little less.
  var accent = 0.0;
  if (input.lift > 0.5) {
    accent = 1.0;
  }
  if (input.impact > 0.0) {
    accent = max(accent, 0.75 * input.impact);
  }
  // The swell is the accent's envelope, fading over 3 s, from the runtime.
  state.swell = max(input.swell_lift, 0.75 * input.swell_hit) * params.swell;
  if (accent > 0.0) {
    if (state.leap <= 0.0 || state.leap >= 1.0) {
      state.leap = 0.001;
      state.leap_power = accent;
    }
  }
  // The sea holds still, and every few beats (and on an accent) a new set of triangles takes its
  // place, as tall as the music is loud then, taller on the kick and in an accent's swell.
  // Without a beat it changes on a hit, at most every second or so.
  let beat_no = u32(input.beat_no);
  let sea_turn = (input.tick > 0.5 && beat_no % max(u32(params.sea_beats), 1u) == 0u) || accent > 0.0 || (onset && t - state.sea_last > 1.2);
  if (sea_turn || frame.first == 1u) {
    state.sea_gen = f32((u32(state.sea_gen) + 1u) % 4096u);
    state.sea_amp = state.wave * (1.0 + params.wave_kick * kin) + 1.2 * state.swell;
    state.sea_last = t;
  }
  state.swell_phase = fract(state.swell_phase - dt * 0.12);
  if (state.leap > 0.0 && state.leap < 1.0) {
    state.leap = min(state.leap + dt / max(params.leap_time, 0.2), 1.0);
  }

  // The director cuts the shot; here only the cue's other work is left. A new kind of section
  // (`pattern` changes; it is -1 where the music has no sections) is patterned its own way, the
  // same each time that kind comes back. The pattern also changes every few bars while the beat
  // is followed. The light is no work of the scene's: the runtime changes the look (`look`).
  if (shot.cuts != u32(state.cuts_seen)) {
    take_shot();
  }
  if (input.pattern != state.pattern_seen) {
    state.pattern_seen = input.pattern;
    if (input.pattern >= 0.0) {
      choose_mode_of(input.pattern);
    }
  } else if (input.cue < 0.5 && input.bar_line > 0.5) {
    let bar = u32(floor(input.bar));
    if (t - state.last_mode > 4.0 && (bar + 2u) % max(u32(params.mode_bars), 1u) == 0u) {
      // Another pattern than the one on, drawn by the bar (the scene's seed alone would draw the
      // same one every time, and it never changed).
      let next = (u32(state.mode) + 1u + u32(random(31u + bar) * 3.0)) % 4u;
      choose_mode(f32(next), random(32u + bar));
    }
  }

  // Gliding as fast as the music's tempo, locked to the beat: a gate passes on every beat (or
  // every two or four, or two a beat, whichever is nearest that pace). `pace` is the tempo over
  // 90 bpm, so `cruise` is the speed at 90 bpm (the old pace multiplier was 1 at a middling song).
  // A quiet passage flies slower and a loud one faster (`energy`), a fade slows it and silence
  // nearly stills it (`audible`). Locked to the beat, a slower flight passes a gate every two beats
  // instead of one, so it stays on the beat.
  let pace = input.pace;
  let by_energy = mix(1.0, mix(0.5, 1.2, input.energy), params.energy_speed);
  let cruise = params.cruise * pace * by_energy * mix(0.2, 1.0, input.audible);
  state.vtarget = cruise + (state.vtarget - cruise) * exp(-dt / 0.6);
  let one = params.spacing * input.tempo / 60.0;
  let want = clamp(round(log2(max(state.vtarget, 0.1) / one)), -2.0, 1.0);
  if (state.per_beat <= 0.0 || (input.bar_line > 0.5 && abs(want - log2(state.per_beat)) > 0.5)) {
    state.per_beat = exp2(want);
  }
  var err = fract(input.clock * state.per_beat) - fract(state.s / params.spacing);
  err -= round(err);
  let want_vel = one * state.per_beat + err * params.spacing / 0.7;
  let goal = mix(state.vtarget, want_vel, locked * params.beat_lock);
  state.vel = rv_approach(state.vel, goal, dt, mix(0.5, 0.2, locked));
  state.s += dt * max(state.vel, 0.0);

  // The spirits. Their centre runs at s, through the middle of each gate as the beat comes.
  let s = state.s;
  let n = clamp(round(state.count + input.more_spirits), 1.0, f32(SPIRITS));
  // How hard they move, by how loud it is this moment (not the section's average): `energy`, the
  // moment's energy spread over its range, which follows the phrases but not every hit, and
  // calmer still as the music fades (`audible`). Quiet, small calm moves; loud, wide and sharp
  // ones. It scales how far they go and how stiffly they spring, never where they are
  // in their figures, so a change of loudness does not make them jump. (Called `drive` here, as a
  // motion scale; it is no input.)
  let drive = mix(1.0, mix(0.35, 1.6, input.energy), params.energy_motion) * mix(0.4, 1.0, input.audible);
  let k = params.stiff * mix(0.7, 1.3, clamp((drive - 0.35) / 1.25, 0.0, 1.0));
  let c = 2.0 * sqrt(k) * params.damp;
  // Through a gate they draw in to its middle; between gates they spread.
  let between = sin(3.14159265 * fract(gate_pos(s)));
  let open = mix(params.pinch, 1.0, between * between);
  // A leap: up into the sky and back down, in an arc.
  let leap_h = select(0.0, sin(3.14159265 * state.leap) * params.leap * state.leap_power, state.leap > 0.0 && state.leap < 1.0);
  // Everything the spirits do is counted in beats: each beat a step, quick off the mark and
  // settled before the next (the runtime's beat clock runs on, at the tempo, without a beat).
  let stepped = input.beat_pos;
  let fb = input.beat_no;
  let snap = stepped - fb;
  let per_bar = floor(input.meter);
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
      let beat_s = 60.0 / max(input.tempo, 1.0);
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
    let swell_k = clamp(0.4 + 0.7 * (input.band_low + input.band_mid) * 0.5, 0.4, 1.8);
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

  // The camera, by its shot, moving on with the spirits; it never sways, nor rises with their
  // leaps. Its roll is the director's dutch angle, fixed within a shot.
  var cam: vec3f;
  var aim_at: vec3f;
  var lens = 1.0;
  // Seen from behind, the spirits are the ends of their trails; their heads and their moves show
  // from the side and from ahead, so the near shots are there.
  if (state.view > 6.5) {
    // The burst's last: high above the line, a little behind and to one side, looking down on
    // the gates winding on ahead and the spirits threading them.
    let cz = s - 4.0;
    cam = vec3f(path_x(cz) + state.lat * 4.0, params.through + 30.0, cz);
    aim_at = vec3f(path_x(s + 12.0), 0.0, s + 12.0);
    lens = 1.0;
  } else if (state.view > 5.5) {
    // The burst's detail: a long lens on their heads from ahead, as they come on.
    let cz = s + 14.0;
    cam = vec3f(path_x(cz) - state.lat * 0.8, params.through + 0.4, cz);
    aim_at = vec3f(path_x(s), params.through + 0.3, s);
    lens = 0.4;
  } else if (state.view > 4.5) {
    // The burst's close: right beside their heads, looking straight across at them, between a
    // gate's pillar and the end of its beams.
    let cz = s + 0.5;
    cam = vec3f(path_x(cz) + state.lat * 3.2, params.through + 0.3, cz);
    aim_at = vec3f(path_x(s + 0.5), params.through + 0.2, s + 0.5);
    lens = 0.85;
  } else if (state.view > 3.5) {
    // The burst's medium: above and behind to one side, the whole formation passing a gate.
    let cz = s - 7.0;
    cam = vec3f(path_x(cz) + state.lat * 6.0, params.through + 4.5, cz);
    aim_at = vec3f(path_x(s + 3.0), params.through, s + 3.0);
    lens = 0.9;
  } else if (state.view > 2.5) {
    // Ahead of the spirits and to one side, outside the gates, looking down at about 20 degrees
    // as they come on through the gates towards it, a slightly long lens on the leaders' heads.
    // (The torii are listed both ways from the camera, by where it looks, so those behind them
    // are there.)
    let cz = s + 20.0;
    cam = vec3f(path_x(cz) + state.lat * 8.0, params.through + 8.1, cz);
    aim_at = vec3f(path_x(s), params.through + 0.3, s);
    lens = 0.55;
  } else if (state.view > 1.5) {
    // Beside them and a little ahead, flying with them and looking back across and down at them
    // at about 20 degrees (the seed picks the side): every trail whole, the formation changing,
    // the gates sweeping past below. Well outside the pillars and the beams' ends.
    let cz = s + 2.5;
    cam = vec3f(path_x(cz) + state.lat * 12.0, params.through + 4.7, cz);
    aim_at = vec3f(path_x(s - 0.5), params.through + 0.2, s - 0.5);
    lens = 0.95;
  } else if (state.view > 0.5) {
    // From behind and to one side, a little above the water and looking slightly down (lower,
    // the sea grazes the view and shows its triangles' faults), on past the spirits to the light
    // far ahead; a slightly long lens.
    let cz = s - 18.0;
    // Out between the line's gates and the strays, clear of both.
    cam = vec3f(path_x(cz) + state.lat * 9.0, params.rear_height, cz);
    aim_at = vec3f(path_x(s + 30.0), 1.0, s + 30.0);
    lens = 0.75;
  } else {
    // High up on the left and well behind the spirits, looking on ahead down the line to the
    // horizon rather than down on them, a wide lens (left is -x, the way the camera looks being
    // +z).
    let cz = s - params.cam_dist * 3.0;
    cam = vec3f(path_x(cz) - params.side_offset * 1.6, params.high, cz);
    aim_at = vec3f(path_x(s + 40.0), 0.0, s + 40.0);
    lens = 1.05;
  }
  state.cam_pos = cam;
  state.cam_fwd = normalize(aim_at - cam);
  state.roll = shot.roll;
  state.lens = lens;
  let right = normalize(cross(vec3f(0.0, 1.0, 0.0), state.cam_fwd));
  state.cam_right = right;
  state.cam_up = cross(state.cam_fwd, right);

  // Each spirit's ribbon and flakes, made in the world and projected for the passes.
  for (var h = 0u; h < SPIRITS; h++) {
    build_ribbon(h);
  }
  list_torii();

  post.bloom_intensity = input.bloom;
  post.exposure = params.exposure;
  // The theme's hue.
  let follow = params.player_colour == 1u && frame.accent.a > 0.5;
  let theme = oklab(select(params.theme.rgb, frame.accent.rgb, follow));
  state.theme_hue = atan2(theme.z, theme.y);
  // The sea in marks fringes a little red and blue, as on a screen.
  post.chromatic = select(0.0, 0.0022, params.sea_look == 0u);
}

// ---- The torii in sight ----
// Once a frame, nearest first: the slots out from the camera's, both ways (up to 160), each with
// its torii of the line and (within 400 m) its two strays, each kept if it may be in the view.
// Both ways always: a view across the line, or down on it, sees strays far ahead and behind
// whichever way it faces (picking the ways by where it faced dropped them).
// Each one in sight is worked out once and kept for the passes, and listed in every tile of the
// screen its box covers (the box its ray test starts with, turned and leaned as it stands, its
// eight corners put on the screen; its bounding sphere when a corner is behind the camera). One
// small on the screen is marked to be drawn plainly.
fn list_torii() {
  for (var k = 0u; k < TILES; k++) {
    state.tile_n[k] = 0u;
  }
  state.torii_n = 0u;
  // (See gate: the highest a foot of the line stands, and how far out of the ball round where it
  // can stand the box of the biggest, 1.8, can reach.)
  let low = max(-0.3, params.through + 1.0 - 4.28 * 0.85);
  let reach = 8.48 * 1.8 + sqrt(params.scatter * params.scatter + 0.25 * (low + 1.8) * (low + 1.8)) + 0.1;
  var cam: Camera;
  cam.pos = state.cam_pos;
  cam.fwd = state.cam_fwd;
  cam.right = state.cam_right;
  cam.up = state.cam_up;
  // The screen is turned by the dutch angle on the camera's plane (see view.wgsl's camera_ray):
  // what may be seen reaches out to the turned rectangle's bounds.
  let ky = 0.5 * state.lens * 0.8;
  let kx = frame.aspect * ky;
  let cr = abs(cos(state.roll));
  let sr = abs(sin(state.roll));
  cam.kx = cr * kx + sr * ky;
  cam.ky = sr * kx + cr * ky;
  let sp = params.spacing;
  let here = floor(state.cam_pos.z / sp);
  for (var k = 0.0; k <= 160.0; k += 1.0) {
    for (var way = 1.0; way >= -1.0; way -= 2.0) {
      if (way < 0.0 && k < 0.5) {
        continue;
      }
      let i = here + k * way;
      // Only a torii that may be in sight is worked out (see may_see): one of the line stands
      // with its feet within `scatter` of the line, from -1.8 up to `low`.
      if (may_see(vec3f(path_x(gate_z(i)), 0.5 * (low - 1.8), gate_z(i)), reach, cam)) {
        see_torii(gate(i), i);
      }
      if (k * sp < 400.0) {
        for (var sd = 0.0; sd < 2.0; sd += 1.0) {
          let j = 2.0 * i + sd;
          let place = stray_place(j);
          // A stray's feet stand from -0.2 down to -1.8, or sunk to 3.64 of its size.
          let deep = min(-3.64 * place.z, -1.8);
          if (place.w > 0.5 && may_see(vec3f(place.x, 0.5 * (deep - 0.2), place.y), 8.48 * place.z + 0.5 * (-0.2 - deep) + 0.1, cam)) {
            see_torii(stray(j), -1.0 - j);
          }
        }
      }
    }
  }
}

// Whether a torii whose box lies within the ball round c of radius r may be listed: every
// corner of its box lies within 8.48 of its size of its feet (turned and leaned any way). False
// only when the ball lies wholly in front of the camera and wholly off one side of the screen: then
// every corner of its box is put on the screen beyond that side, and see_torii passes it by
// (it is not worth working out the torii to find that).
fn may_see(c: vec3f, r: f32, cam: Camera) -> bool {
  let q = c - cam.pos;
  let z = dot(q, cam.fwd);
  // Wholly behind the camera: out of sight (the line both ways is asked about, so half of it is).
  if (z + r < 0.0) {
    return false;
  }
  if (z - r < 0.3) {
    return true;
  }
  return abs(dot(q, cam.right)) - cam.kx * z <= r * sqrt(1.0 + cam.kx * cam.kx) && abs(dot(q, cam.up)) - cam.ky * z <= r * sqrt(1.0 + cam.ky * cam.ky);
}

// The camera as may_see takes it: kx and ky how far off its axis the screen's edges are, across
// and up, for each metre ahead.
struct Camera {
  pos: vec3f,
  fwd: vec3f,
  right: vec3f,
  up: vec3f,
  kx: f32,
  ky: f32,
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
    // On the screen, which is the camera's plane turned back by the dutch angle: the tiles are
    // the screen's (left unturned, a torii was listed in tiles beside the ones it covers, and
    // cut off along their edges in a tilted shot).
    let s = rv_rotate(focal * vec2f(dot(q, state.cam_right), dot(q, state.cam_up)) / qz, -state.roll);
    lo_s = min(lo_s, s);
    hi_s = max(hi_s, s);
  }
  if (whole) {
    // A corner behind the camera: the bounding sphere's circle instead, or the whole screen.
    var c = vec2f(0.0);
    var rad = 10.0;
    if (zc > r * 1.05) {
      c = rv_rotate(focal * vec2f(dot(v, state.cam_right), dot(v, state.cam_up)) / zc, -state.roll);
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
  let live = mix(1.0, pow(clamp(0.2 + 0.8 * band(h % 3u), 0.2, 2.1), 1.5), params.react);
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
