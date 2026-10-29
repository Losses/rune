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
// which and when (see `[[shot]]` and `[[burst]]` in scene.toml); the scene only knows how each
// looks. Views: 0 straight on, 1 from one side looking along the wall, 2 close up through a long
// lens, 3 looking up the wall (shot.index 0 to 3), and 4 for a burst's close-ups (shot.index 4 to
// 7, shot.fill 1 to 4), each framed on a set the scene chose (choose_burst).
fn choose_view() {
  if (shot.fill > 0u) {
    choose_burst();
    return;
  }
  state.hero_on = 0.0;
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
    state.cam.z = -params.distance * 0.85;
    state.yaw = way * 0.12;
    state.pitch = (r3 - 0.5) * 0.06;
    state.lens = 0.3;
    state.dir = way;
  } else {
    // Looking up the wall: near it, pitched up 30 to 40 degrees, so the piles converge upward into
    // the dark; a little turned, and a third of the director's dutch angle (30 to 100 % of 0.3 rad
    // on a medium shot, so about 2 to 6 degrees, never none). The plan centres where the view meets
    // the wall (plan_look), which is higher up the steeper it looks, and is big enough for the
    // whole frame.
    state.cam.z = -params.distance * 0.6;
    state.yaw = way * 0.05;
    state.pitch = 0.52 + 0.17 * r3;
    state.lens = 0.6;
    state.roll = shot.roll * 0.33;
    state.dir = way;
  }
  // Somewhere new on the wall (it has no end either way).
  state.cam.x += (40.0 + 200.0 * random(5u)) * select(-1.0, 1.0, random(6u) < 0.5);
  state.cam.y = 1.0 + 60.0 * random(7u);
}

// The set at x, y on the wall, worked out (the views read it from the plan instead).
fn set_here(x: f32, y: f32) -> Set {
  let b = floor(x / BAY);
  let lift = bay_lift(b);
  let k = floor((y - lift) / BLOCK);
  let c = column_in(bay_of(b, k), b, k, x);
  return set_at(c, pile_of(c), y);
}

// Whether a set will do for a burst's shot: there, with a screen big enough to be the subject (the
// little monitor's is not), and, for the two shots turned to one side (`yaw`), no neighbour on the
// camera's side standing out further than it, which would hide the screen's near edge.
fn hero_suits(s: Set, fill: u32, yaw: f32) -> bool {
  if (!s.on) {
    return false;
  }
  let m = model_of(s.model);
  if (m.screen.w - m.screen.y < 0.3) {
    return false;
  }
  if (fill <= 2u) {
    let front = s.centre.z - s.half.z;
    for (var i = 0u; i < 2u; i++) {
      let n = set_here(s.centre.x - sign(yaw) * (s.half.x + 0.12 + 0.35 * f32(i)), s.centre.y);
      if (n.on && n.centre.z - n.half.z < front - 0.03) {
        return false;
      }
    }
  }
  return true;
}

// A burst's shot: the four close-ups of the burst, each on one set the scene chooses and whose
// screen it sets, so the subject is always in frame and is not looked for in the picture. At each
// cut the camera goes somewhere new on the wall as choose_view does, finds a set there that suits
// the shot (a few nearby points are tried), and is put in front of it at the distance that gives
// the framing, each shot nearer than the one before:
//   1  close: turned about 20 degrees, the whole set in frame; its screen shows the wave
//   2  close: turned about 25 degrees the other way, nearer, the set filling the frame; the
//      stereo picture
//   3  detail: straight on, the screen about 70 % of the picture's height; a cover from the
//      library
//   4  detail: straight on, the screen filling the picture; the cover of the song playing
// (the two close-ups of the screen are both covers). The picture's share of the screen or set is
// its height over the picture's, 2 * lens * distance; a dutch angle turns the frame, so the first
// three are made
// smaller until the turned box is inside it, which is also what keeps them whole in a picture
// narrower than they are (a phone held upright). The last is meant to fill the frame, by its
// height or, in such a picture, by its width, and a corner of it goes past the frame when the
// picture is turned.
// What each of the burst's shots shows on its set's screen, by fill (the kinds of hero_picture in
// view.wgsl: 0 the wave, 1 a library cover, 2 the stereo picture, 3 the cover of the song playing).
const BURST_SHOWS = array<u32, 4>(0u, 2u, 1u, 3u);

fn choose_burst() {
  let fill = shot.fill;
  state.view = 4.0;
  state.roll = shot.roll;
  let way = select(-1.0, 1.0, random(2u) < 0.5);
  // The shot turns to one side and the next to the other.
  if (fill == 1u || state.side == 0.0) {
    state.side = way;
  } else if (fill == 2u) {
    state.side = -state.side;
  }
  var yaw = 0.0;
  var lens = 0.3;
  if (fill == 1u) {
    yaw = state.side * 0.35;
  } else if (fill == 2u) {
    yaw = state.side * 0.44;
  } else {
    lens = 0.25;
  }
  state.yaw = yaw;
  state.pitch = (random(3u) - 0.5) * 0.04;
  state.lens = lens;
  state.dir = way;
  state.hero_kind = f32(BURST_SHOWS[fill - 1u]);
  state.hero_pick = random(4u);

  // Somewhere new on the wall, and a set there that suits the shot.
  let place = vec2f(state.cam.x + (40.0 + 200.0 * random(5u)) * select(-1.0, 1.0, random(6u) < 0.5), 1.0 + 60.0 * random(7u));
  var s: Set;
  var found = false;
  for (var i = 0u; i < 24u; i++) {
    let c = set_here(place.x + (random(20u + i) - 0.5) * 6.0, place.y + (random(60u + i) - 0.5) * 4.0);
    if (hero_suits(c, fill, yaw)) {
      s = c;
      found = true;
      break;
    }
  }
  state.hero_on = f32(found);
  state.hero_id = s.id;
  state.push = 0.0;
  if (!found) {
    // No set to frame (the wall is hardly ever so bare): look at the place from the front view's
    // distance, as the ordinary views do.
    state.aim = vec3f(place, -0.5);
    state.reach = params.distance * 0.85;
    state.cam = state.aim - vec3f(sin(yaw), 0.0, cos(yaw)) * state.reach;
    return;
  }

  // The front of the set, and what the shot looks at: its middle, or its screen's.
  let m = model_of(s.model);
  let low = s.centre - s.half;
  let front = s.centre.z - s.half.z;
  let screen_at = vec3f(low.x + 0.5 * (m.screen.x + m.screen.z), low.y + s.plinth + 0.5 * (m.screen.y + m.screen.w), front);
  let screen_box = vec2f(m.screen.z - m.screen.x, m.screen.w - m.screen.y);
  let set_at_front = vec3f(s.centre.xy, front);
  var box = screen_box;
  var share = 0.7;
  state.aim = screen_at;
  if (fill == 1u) {
    box = 2.0 * s.half.xy;
    share = 0.55;
    state.aim = set_at_front;
  } else if (fill == 2u) {
    box = 2.0 * s.half.xy;
    share = 0.62;
    state.aim = mix(set_at_front, screen_at, 0.4);
  } else if (fill == 4u) {
    // Filling the picture's height, or its width where the picture is narrower than the screen
    // (a phone held upright).
    share = min(0.95, 0.96 * frame.aspect * box.y / box.x);
  }
  if (fill <= 3u) {
    // Keep the turned box (its width over its height `ratio`) inside the frame, with a margin.
    let ratio = box.x / box.y;
    let c = cos(state.roll);
    let t = abs(sin(state.roll));
    let tall = 0.5 * share * (c + ratio * t);
    let wide = 0.5 * share * (ratio * c + t);
    share *= min(1.0, min(0.46 / tall, 0.46 * frame.aspect / wide));
  }
  state.reach = box.y / (2.0 * lens * share);
  let f = vec3f(sin(yaw) * cos(state.pitch), sin(state.pitch), cos(yaw) * cos(state.pitch));
  state.cam = state.aim - f * state.reach;
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
  // The way the screens change channel, by the kind of section (the `way` input picks it from the
  // section's label, so every chorus the same way). Read only here, so it never changes within a
  // shot; the parameter `change_way` may pin one, which the view reads at once.
  state.way = clamp(floor(input.way + 0.5), 0.0, 4.0);
  // How much of the covers' weight the channels use: the grey look keeps a share of it (most
  // screens then show snow or a scope), the theme colour look all. Decided here, at the cut, and
  // never from the look's fade, which moves every frame: channel() is worked out again every frame
  // and a weight that moved would make screens jump channel between cuts. A look changes at a
  // section start, which the director normally cuts at; one that comes without a cut (no chart, or
  // a cut held back) leaves the weight as it was until the next cut.
  state.cover_w = select(1.0, clamp(params.cover_grey, 0.0, 1.0), look.to == 0u);
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
  // The theme colour look's palette, in the same hue: a dark shade up to near white (Neon Street's
  // stops). The grade takes the colour from it and keeps the picture's own light.
  state.theme_ramp[0] = vec4f(max(rv_oklab_to_linear(vec3f(0.12, 0.03 * cos(h), 0.03 * sin(h))), vec3f(0.0)), 1.0);
  state.theme_ramp[1] = vec4f(max(rv_oklab_to_linear(vec3f(0.36, 0.14 * cos(h), 0.14 * sin(h))), vec3f(0.0)), 1.0);
  state.theme_ramp[2] = vec4f(max(rv_oklab_to_linear(vec3f(0.62, 0.19 * cos(h), 0.19 * sin(h))), vec3f(0.0)), 1.0);
  state.theme_ramp[3] = vec4f(max(rv_oklab_to_linear(vec3f(0.95, 0.03 * cos(h), 0.03 * sin(h))), vec3f(0.0)), 1.0);
  // How far the picture is in the theme colour's look: the runtime's fade from look.prev to look.to
  // (0 the grey look, 1 the colour one). The scene has no setting that pins a look: the lab holds
  // one.
  state.colour = select(0.0, 1.0 - look.mix, look.prev == 1u) + select(0.0, look.mix, look.to == 1u);
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
  // side faster, close up slower (a burst's close-ups do not cruise, below). The cruise is for 90
  // bpm (`pace`), and goes with how much is heard (smoothed, so it slows with a fade) and stops in
  // silence.
  let cruise = params.cruise * input.pace * (1.0 + params.energy_speed * (mix(0.6, 1.4, input.energy) - 1.0)) * input.audible;
  state.vtgt = cruise + (state.vtgt - cruise) * exp(-dt / max(params.ease, 0.05)) + f32(stepped) * params.push * cruise;
  if (fresh) {
    state.vel = state.vtgt;
  }
  state.vel += (state.vtgt - state.vel) * (1.0 - exp(-dt / max(params.accel, 0.01)));
  // The views' own speeds: straight on 1, from the side 1.6, close 0.45, looking up 1.2 (near the
  // wall, so the same speed sweeps the picture faster).
  var view_speed = 1.0;
  if (state.view > 0.5 && state.view < 1.5) {
    view_speed = 1.6;
  } else if (state.view > 1.5 && state.view < 2.5) {
    view_speed = 0.45;
  } else if (state.view > 2.5 && state.view < 3.5) {
    view_speed = 1.2;
  }
  if (state.view > 3.5) {
    // A burst's close-up does not cruise (a long lens would carry its screen out of frame in a
    // beat): it pushes in slowly towards what it looks at, a few percent of the way over a second,
    // eased, and still in silence.
    state.push = min(state.push + dt * input.audible, 1.0);
    let f = vec3f(sin(state.yaw) * cos(state.pitch), sin(state.pitch), cos(state.yaw) * cos(state.pitch));
    let eased = 1.0 - (1.0 - state.push) * (1.0 - state.push);
    state.cam = state.aim - f * state.reach * (1.0 - 0.04 * eased);
  } else {
    state.cam.x += dt * state.dir * state.vel * view_speed;
  }

  // The snow's bar rolls on as much as the music is heard.
  state.bar += dt * input.audible;

  set_colours();
  set_scope();

  post.exposure = params.exposure;
  post.bloom_intensity = params.glow;
  post.grain = params.grain;
  post.chromatic = 0.0;
}
