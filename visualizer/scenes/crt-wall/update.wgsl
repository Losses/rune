// Once per frame: the beat length the screens change channel by (the clock itself is the
// runtime's), the camera travelling along the wall as fast as the music sounds (pushed on each
// beat), the views following the director's cuts, the phosphor's colour from the player's, and
// the scopes' trigger and gain.

// A number from 0 to 1 for this cut, `n` telling the uses apart: made from the shot's seed from
// the runtime's director, which is fixed within a shot and new at each cut.
fn random(n: u32) -> f32 {
  return rv_unit(u32(shot.seed * 16777215.0) * 2654435761u + n);
}

// Frame the shot the director cut to, and where on the wall to see it from. The director chooses
// which and when (see `[[shot]]` in scene.toml); the scene only knows how each looks. Views: 0
// straight on, 1 from one side looking along the wall, 2 far off, 3 close up through a long lens.
fn choose_view() {
  let r2 = random(2u);
  let r3 = random(3u);
  let v = f32(min(shot.index, 3u));
  state.view = v;
  state.roll = shot.roll;
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
  state.gain += (min(want, 40.0) - state.gain) * (1.0 - exp(-rv_dt() / 0.3));
}

fn update() {
  let dt = rv_dt();
  let t = frame.time;
  if (frame.first == 1u) {
    state.cam = vec3f(0.0, 10.0, -params.distance);
    state.ens = 1.0;
    state.gain = 4.0;
    state.cuts = shot.cuts;
    choose_view();
    state.vel = params.cruise;
    state.vtgt = params.cruise;
  }

  // The runtime keeps the beat clock and says how far the beat is followed (`locked`). The screens
  // change channel on the clock, so a beat length in seconds is all the view needs besides it.
  // Without a followed beat the screens keep their channels (their flashes and jolts fade with
  // `locked`) and the hits of the music stand in for the beat in the camera's push.
  state.beat_len = 60.0 / input.tempo;
  let beat = input.beat > 0.5;

  // The energy the speed follows: the slow drive, eased once more.
  state.ens = rv_approach(state.ens, 0.5 + 1.5 * input.drive, dt, 0.6);

  // The channels group anew at a section start, from the section's number, so every chorus is
  // grouped the same way; at any other cut of the view they group anew from the number moved on.
  let section_start = input.section_start > 0.5;
  if (section_start) {
    state.section = input.section;
  }
  // The director cuts: take its shot as the view. The camera jumps along the wall, so the new
  // view starts at the speed the music sounds now.
  var fresh = false;
  if (shot.cuts != state.cuts) {
    state.cuts = shot.cuts;
    choose_view();
    if (!section_start) {
      state.section += 101.0;
    }
    fresh = true;
  }
  // Some sets are always off, and more in a quiet section.
  state.dark = params.off_always + params.off * pow(max(1.0 - input.drive, 0.0), 1.5);

  // How fast the music sounds, as a pace against a song of middling tempo: the camera's cruise
  // is for 90 bpm, a faster song goes faster and a slower one slower.
  let pace = clamp(input.tempo / 90.0, 0.25, 3.0);

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
