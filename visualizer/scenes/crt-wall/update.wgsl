// Once per frame: the step the screens change channel by, the camera travelling along the wall as
// fast as the music sounds (pushed on each step, still in silence), the views following the
// director's cuts, what the cut decides for the wall, the phosphor's colour from the player's, and
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

// What a cut of the view decides for the wall, so that nothing on it pops between cuts: how many
// screens are off and how the channels group. The picture changes anyway at a cut.
fn decide_wall() {
  // Some sets are always off, and more in a quiet part of the song: `section_level` (0 quiet to 1
  // high, as the director's tiers read it) takes the share from all off_always + off down to
  // off_always along (1 - level)^1.5, the curve the drive had.
  state.dark = params.off_always + params.off * pow(max(1.0 - input.section_level, 0.0), 1.5);
  // The channels group anew at each cut, from the section's number and the cuts so far (so a
  // section with several cuts, and a song with no sections to tell, group differently each time).
  state.section = input.section + 101.0 * f32(shot.cuts);
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
    decide_wall();
    state.vel = params.cruise;
    state.vtgt = params.cruise;
    // No change is rolling when the scene begins.
    state.step_age = 1000.0;
  }

  // The screens' step: the count goes on by one on a beat (the music's hits where no beat is
  // followed), but only if half a beat has passed since the last step (hits can be dense) and
  // something is heard (`audible`), so that a stray onset in near silence moves nothing and a
  // song's fade-out and silence leave every screen on its channel. The screens change channel on
  // the count and roll for `change_time` seconds after it; the camera is pushed on it.
  let beat_len = 60.0 / (90.0 * input.pace);
  state.step_age = min(state.step_age + dt, 1000.0);
  let heard = input.audible > 0.3;
  let stepped = input.beat > 0.5 && heard && state.step_age >= 0.5 * beat_len;
  if (stepped) {
    state.count += 1.0;
    state.step_age = 0.0;
  }

  // The energy the speed follows: the slow drive, eased once more.
  state.ens = rv_approach(state.ens, input.energy, dt, 0.6);

  // The director cuts: take its shot as the view, and decide the wall's off screens and grouping
  // (nothing else changes them). The camera jumps along the wall, so the new view starts at the
  // speed the music sounds now.
  var fresh = false;
  if (shot.cuts != state.cuts) {
    state.cuts = shot.cuts;
    choose_view();
    decide_wall();
    fresh = true;
  }

  // The camera glides along the wall at a cruise, pushed on each step and easing back; from the
  // side faster, close up slower. The cruise is for 90 bpm (`pace`), and goes with how much is
  // heard (smoothed, so it slows with a fade) and stops in silence.
  let cruise = params.cruise * input.pace * (1.0 + params.energy_speed * (state.ens - 1.0)) * input.audible;
  state.vtgt = cruise + (state.vtgt - cruise) * exp(-dt / max(params.ease, 0.05)) + f32(stepped) * params.push * cruise;
  if (fresh) {
    state.vel = state.vtgt;
  }
  state.vel += (state.vtgt - state.vel) * (1.0 - exp(-dt / max(params.accel, 0.01)));
  let view_speed = select(select(select(1.0, 0.45, state.view > 2.5), 1.6, state.view > 0.5 && state.view < 1.5), 1.0, state.view < 0.5);
  state.cam.x += dt * state.dir * state.vel * view_speed;

  // The snow's bar rolls on as much as the music is heard.
  state.bar += dt * input.audible;

  set_colours();
  set_scope();

  post.exposure = params.exposure;
  post.bloom_intensity = params.glow;
  post.grain = params.grain;
  post.chromatic = 0.0;
}
