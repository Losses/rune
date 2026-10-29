// Once per frame: the camera flies down the tunnel as fast as the beat (and the energy), surging
// on each counted hit; the badges' crests turn a step on it; the emoji move to the melody, each in
// its own way; now and then (on a cue) the camera glides to another place to film from, and the
// badges coming change colour. All of it stands still in silence and slows with a fade (the phase
// and the flight go by how much is heard). Then everything in the slots in sight is placed,
// turned, projected and sorted into the screen's tiles, nearest first.

// ---- Colour ----
// The tunnel's colours are hand made, each with its own hue. Following the player's colour (its
// accent, which follows the system, the listener's choice or the cover), every one keeps how light
// and how colourful it is (held down, so it is never loud) and takes the player's hue, those that
// were of several colours turned a little either side of it so they still differ. In OKLab,
// where a step in lightness or hue looks as big as it is.

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

// The colours as made, by number: 0, 1 the tunnel's depths (far, near); 2 the core's light, 3 its
// halo, 4 its rays; 5 the speed lines; 6–9 the glints (white, cyan, pink, gold); 10 the haze;
// 11 the sparkles; 12–14 the lacquers (black, vermilion, indigo); 15, 16 the leaves (gold,
// silver); 17, 18 what the tunnel mirrors (below, above), 19 the core in it, 20 the light behind,
// 21, 22 the strip lights; 23 the key light's highlight.
fn colour_made(k: u32) -> vec3f {
  var c = array<vec3f, 24>(
    vec3f(0.002, 0.003, 0.01), vec3f(0.006, 0.012, 0.04), vec3f(0.6, 0.75, 1.0), vec3f(0.12, 0.2, 0.6), vec3f(0.5, 0.65, 1.0),
    vec3f(0.7, 0.8, 1.0), vec3f(1.0, 1.0, 1.0), vec3f(0.4, 0.9, 1.0), vec3f(1.0, 0.5, 0.8), vec3f(1.0, 0.8, 0.35),
    vec3f(0.35, 0.22, 0.9), vec3f(0.85, 0.9, 1.0), vec3f(0.012, 0.011, 0.012), vec3f(0.42, 0.04, 0.02), vec3f(0.02, 0.04, 0.12),
    vec3f(1.0, 0.74, 0.36), vec3f(0.86, 0.88, 0.92), vec3f(0.01, 0.015, 0.04), vec3f(0.08, 0.1, 0.22), vec3f(0.7, 0.82, 1.0),
    vec3f(0.85, 0.88, 1.0), vec3f(1.0, 0.98, 0.95), vec3f(0.8, 0.9, 1.0), vec3f(1.0, 0.97, 0.92));
  return c[k];
}

// How far each turns from the player's hue: the glints and the haze, which were of their own
// colours, a little either side.
fn hue_offset(k: u32) -> f32 {
  var o = array<f32, 24>(0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, -0.25, 0.3, 0.15, -0.15, 0.0, 0.0, 0.2, -0.1, 0.1, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
  return o[k];
}

fn set_colours() {
  let follow = params.player_colour == 1u && frame.accent.a > 0.5;
  let theme = oklab(frame.accent.rgb);
  let h = atan2(theme.z, theme.y);
  for (var k = 0u; k < 24u; k++) {
    var c = colour_made(k);
    if (follow) {
      let lab = oklab(c);
      c = oklch(lab.x, min(length(lab.yz), 0.12 * params.chroma), h + hue_offset(k));
    }
    state.colours[k] = vec4f(c, 0.0);
  }
}

fn random(n: u32) -> f32 {
  return rv_unit(frame.seed * 2654435761u + n);
}

// A turn: local x, y and z in the world, after turning by c about z, then a about x, then b
// about y. Its rows are what take a point into the thing's own frame.
fn turned(a: f32, b: f32, c: f32) -> mat3x3f {
  var m = mat3x3f(vec3f(cos(c), sin(c), 0.0), vec3f(-sin(c), cos(c), 0.0), vec3f(0.0, 0.0, 1.0));
  for (var k = 0; k < 3; k++) {
    let v = m[k];
    let x = vec3f(v.x, v.y * cos(a) - v.z * sin(a), v.y * sin(a) + v.z * cos(a));
    m[k] = vec3f(x.x * cos(b) + x.z * sin(b), x.y, -x.x * sin(b) + x.z * cos(b));
  }
  return m;
}

// A badge's look, packed in one number: its lacquer (0 black, 1 vermilion, 2 indigo) and its
// leaf (8 × 0 gold, 1 silver), by the section's palette: 0 black lacquer and gold, a quarter of
// them vermilion; 1 vermilion and gold, a quarter black; 2 indigo and silver, a quarter black.
fn look_of(palette: f32, id: vec2f) -> f32 {
  let accent = hash(id + 12.1) < 0.25;
  var body = select(0.0, 1.0, accent);
  var leaf = 0.0;
  if (palette > 1.5) {
    body = select(2.0, 0.0, accent);
    leaf = 1.0;
  } else if (palette > 0.5) {
    body = select(1.0, 0.0, accent);
  }
  return body + 8.0 * leaf;
}

// v turned by angle t about the unit axis a.
fn turn(v: vec3f, a: vec3f, t: f32) -> vec3f {
  return v * cos(t) + cross(a, v) * sin(t) + a * dot(a, v) * (1.0 - cos(t));
}

// The last note of the melody this emoji answered (each answers about `dance` of them, different
// ones for different emoji): how long ago, and how strong; long ago if none of the last eight.
fn note_for(id: vec2f) -> vec2f {
  let count = u32(state.note_count);
  for (var k = 0u; k < 8u; k++) {
    if (k >= count) {
      break;
    }
    let n = count - k;
    if (hash(id + vec2f(f32(n) * 1.618, 3.1)) < params.dance) {
      let e = state.notes[n % 8u];
      return vec2f(frame.time - e.x, e.y);
    }
  }
  return vec2f(1e3, 0.0);
}

// Where slot n's share and colours are kept.
fn slot_cell(n: f32) -> u32 {
  return u32(((i32(n) % 128) + 128) % 128);
}

// Thing q of slot `slot`, into place i: where it is, how it is turned, what it is.
fn place(i: u32, slot: f32, q: u32) {
  let id = vec2f(slot, f32(q) * 17.0);
  // The scene's own time (it stands still in silence), for everything that turns of itself.
  let t = state.phase;
  // What it is: the first of a slot's three a badge or nothing, the second a badge, a cube or
  // nothing, the third a sparkle, a cube or nothing.
  // (How many badges or emoji: as many as the slot's share, set by the music, allows.)
  let h = hash(id + 0.7);
  let given = state.slot_set[slot_cell(slot)];
  let badges = params.badges * given.x;
  var kind = -1.0;
  if (q == 0u) {
    kind = select(-1.0, BADGE, h < badges);
  } else if (q == 1u) {
    kind = select(select(-1.0, CUBE, h < 0.5 * badges + 0.3 * params.cubes), BADGE, h < 0.5 * badges);
  } else {
    kind = select(select(-1.0, CUBE, h < params.sparks + 0.2 * params.cubes), SPARK, h < params.sparks);
  }
  if (kind == BADGE && params.content == 0u) {
    kind = EMOJI;
  }
  if (kind < 0.0) {
    state.pos[i] = vec4f(0.0, 0.0, -1e6, 0.0);
    state.rx[i] = vec4f(1.0, 0.0, 0.0, -10.0);
    return;
  }
  let z = (slot + hash(id + 1.1)) * SLOT_LEN;
  let th = hash(id + 2.2) * 6.2831853;
  let hr = sqrt(hash(id + 3.3));
  var r: f32;
  var size: f32;
  if (kind == BADGE || kind == EMOJI) {
    r = mix(params.inner, params.outer, hr);
    // Two sizes, the small one more often.
    size = select(0.5, 0.85, hash(id + 4.4) > 0.65) * params.badge_size;
  } else if (kind == CUBE) {
    r = mix(1.0, params.outer + 3.0, hr);
    size = mix(0.05, 0.12, hash(id + 4.4));
  } else {
    r = mix(0.6, params.outer + 5.0, hr);
    size = mix(0.07, 0.2, hash(id + 4.4));
  }
  state.pos[i] = vec4f(r * cos(th), r * sin(th), z, size);

  // Badges face the way the camera comes, tipped a little each its own way; a few tumble over
  // and over. (On a counted hit it is the crest that turns, within its frame: see badge.wgsl.) Gold
  // flakes tumble.
  var m: mat3x3f;
  var shape = 0.0;
  if (kind == BADGE) {
    let ht = hash(id + 5.5);
    var b = (hash(id + 6.6) - 0.5) * 1.1;
    if (ht < params.tumble) {
      b += t * mix(0.6, 1.6, hash(id + 7.7)) * select(-1.0, 1.0, hash(id + 7.9) < 0.5);
    }
    let a = (hash(id + 8.8) - 0.5) * 1.0;
    let c = (hash(id + 10.1) - 0.5) * 0.5;
    m = turned(a, b, c);
    shape = 6.0;
  } else if (kind == EMOJI) {
    // An emoji faces the camera square on, turning slowly in the picture's plane, each at its
    // own pace and either way; on a counted hit it rocks a little further and back. To the
    // melody, each in its own way, on the notes it answers: 0 swelling and back, 1 spinning a
    // whole turn round, 2 flipping over side to side, 3 flipping over top to bottom, 4 hopping up
    // and down again, 5 rocking side to side, dying away. The swell, the hop and the rock as big
    // as the note is strong; a spin or a flip always a whole turn, easing in and out.
    let way = select(-1.0, 1.0, hash(id + 9.9) < 0.5);
    var c = hash(id + 10.1) * 6.2831853 + way * (t * params.spin * mix(0.4, 1.2, hash(id + 7.7)) + params.rock * state.kick);
    let act = floor(hash(id + 14.2) * 6.0);
    let heard = note_for(id);
    let p = heard.x / params.move_time;
    let done = clamp(p, 0.0, 1.0);
    let eased = done * done * (3.0 - 2.0 * done);
    var flip = vec2f(0.0);
    if (act < 0.5) {
      size *= 1.0 + 0.6 * heard.y * sin(3.14159265 * clamp(p * 1.6, 0.0, 1.0));
    } else if (act < 1.5) {
      c += way * 6.2831853 * (1.0 - pow(1.0 - done, 3.0));
    } else if (act < 2.5) {
      flip.y = way * 6.2831853 * eased;
    } else if (act < 3.5) {
      flip.x = way * 6.2831853 * eased;
    } else if (act < 4.5) {
      state.pos[i] = vec4f(state.pos[i].xyz + state.cam_up * size * 0.8 * heard.y * sin(3.14159265 * clamp(p * 1.4, 0.0, 1.0)), size);
    } else {
      flip.y = 0.7 * heard.y * sin(done * 9.4247780) * (1.0 - done);
    }
    var ax = state.cam_right * cos(c) + state.cam_up * sin(c);
    var ay = state.cam_up * cos(c) - state.cam_right * sin(c);
    var az = state.cam_fwd;
    ax = turn(ax, ay, flip.y);
    az = turn(az, ay, flip.y);
    ay = turn(ay, ax, flip.x);
    az = turn(az, ax, flip.x);
    m = mat3x3f(ax, ay, az);
    state.pos[i].w = size;
  } else {
    m = turned(hash(id + 5.5) * 6.28 + t * 1.3, hash(id + 6.6) * 6.28 + t * 0.9, hash(id + 8.8) * 6.28);
  }
  state.rx[i] = vec4f(m[0], kind * 10.0 + shape);
  state.ry[i] = vec4f(m[1], look_of(select(given.y, f32(params.palette) - 1.0, params.palette > 0u), id));
  // Its crest (of CRESTS), and which way it turns on the beat (16 for the other way); or its
  // emoji (of 64).
  let crest = floor(hash(id + 13.1) * f32(CRESTS));
  var last = crest + select(0.0, 16.0, hash(id + 9.9) < 0.5);
  if (kind == EMOJI) {
    last = floor(hash(id + 13.1) * 64.0);
  }
  state.rz[i] = vec4f(m[2], last);
}

// Where thing i is on the screen, and which tiles list it.
fn project(i: u32) {
  let p = state.pos[i];
  let kind = floor(state.rx[i].w / 10.0);
  if (p.w <= 0.0) {
    state.scr[i] = vec4f(0.0, 0.0, -1.0, -1.0);
    return;
  }
  let v = p.xyz - state.cam_pos;
  let zc = dot(v, state.cam_fwd);
  // A sphere round it: a badge reaches out to its corners and past its thickness.
  let bound = p.w * select(select(1.0, 1.42, kind == EMOJI), 1.25, kind == BADGE);
  // (A sparkle right by the lens is not drawn: its glow, spread over the whole screen, would
  // white it out; nor a flake, which a camera off the middle may pass close by.)
  if (zc < -bound || (kind == SPARK && zc < 1.0) || (kind == CUBE && zc < 0.8)) {
    state.scr[i] = vec4f(0.0, 0.0, -1.0, -1.0);
    return;
  }
  let aspect = frame.aspect;
  var c = vec2f(0.0);
  var rad = 10.0;
  if (zc > bound * 1.05) {
    c = state.focal * vec2f(dot(v, state.cam_right), dot(v, state.cam_up)) / zc;
    rad = state.focal * bound / sqrt(zc * zc - bound * bound);
    if (kind == SPARK) {
      // A sparkle's rays reach well past its core; and it is never less than a few pixels.
      rad = max(rad * 3.0, 4.0 / frame.resolution.y);
    }
  }
  state.scr[i] = vec4f(c, rad, zc);
  if (abs(c.x) - rad > aspect * 0.5 || abs(c.y) - rad > 0.5) {
    return;
  }
  let lo = screen_uv(c + vec2f(-rad, rad), aspect);
  let hi = screen_uv(c + vec2f(rad, -rad), aspect);
  let x0 = u32(clamp(floor(lo.x * f32(TX)), 0.0, f32(TX - 1u)));
  let x1 = u32(clamp(floor(hi.x * f32(TX)), 0.0, f32(TX - 1u)));
  let y0 = u32(clamp(floor(lo.y * f32(TY)), 0.0, f32(TY - 1u)));
  let y1 = u32(clamp(floor(hi.y * f32(TY)), 0.0, f32(TY - 1u)));
  for (var ty = y0; ty <= y1; ty++) {
    for (var tx = x0; tx <= x1; tx++) {
      let k = ty * TX + tx;
      let n = state.tile_n[k];
      if (n < LIST) {
        state.tile[k * LIST + n] = i;
        state.tile_n[k] = n + 1u;
      }
    }
  }
}

// ---- The camera ----
// Six places to film from, each mirrored by the side, in the order of the shots in scene.toml:
// 0 down the middle, the core on a third, a little above; 1 lower, looking up, the emoji going by
// overhead; 2 off to one side, looking across at the far side's stream, so that none goes by
// close; 3 rolled, near the middle but turned a quarter radian aside, so that the core is off
// centre and the picture leans; 4 a long lens, the emoji stacked one behind another, flattened;
// 5 turned round, the emoji going away into the dark behind. Never straight along the tunnel (the
// core dead centre reads flat), and never further off the middle than the emoji leave clear.
// `roll` is the shot's dutch angle: the rolled place takes a third of it on top of its own roll
// (the rest take all of it); a place chosen by hand (`locked`) has none, and its fourth is the
// older, flatter pose with its harder roll (0.62, the yaw only 0.07).
fn pose_of(view: u32, side: f32, roll: f32, locked: bool) -> Pose {
  var turn = array<vec4f, 6>(
    vec4f(0.42, -0.14, 0.1, 1.0), vec4f(0.18, 0.3, -0.14, 1.0), vec4f(0.7, 0.1, 0.3, 1.0),
    select(vec4f(0.25, 0.06, 0.55, 1.0), vec4f(0.07, 0.06, 0.62, 1.0), locked), vec4f(0.15, -0.08, 0.05, 1.7), vec4f(2.76, -0.06, 0.12, 1.0));
  var at = array<vec2f, 6>(vec2f(0.0), vec2f(0.0, -0.6), vec2f(-0.7, -0.15), vec2f(0.0), vec2f(0.0, 0.35), vec2f(0.0, 0.2));
  let q = turn[view];
  // The dutch angle rides in the unused z of `at`, so it glides with the rest of the pose.
  let tilt = select(roll, roll * 0.33, view == 3u);
  return Pose(vec4f(q.x * side, q.y, q.z * side, q.w), vec4f(at[view].x * side, at[view].y, tilt, 0.0));
}

// Where the camera is now, on its way from where it set off to where it is going: easing out
// and in, slowest at either end.
fn pose_now(t: f32) -> Pose {
  let to = pose_of(u32(state.view), state.side, state.roll_to, params.view > 0u);
  let k = clamp((t - state.last_view) / max(state.glide, 1e-3), 0.0, 1.0);
  let e = k * k * k * (k * (k * 6.0 - 15.0) + 10.0);
  return Pose(mix(state.set_off.turn, to.turn, e), mix(state.set_off.at, to.at, e));
}

fn update() {
  let dt = rv_dt();
  let t = frame.time;
  if (frame.first == 1u) {
    state.s = 0.0;
    state.speed = input.flight;
    // No hit has been counted yet.
    state.step_age = 1000.0;
  }

  // How much is heard, and the scene's time by it: the phase goes on by `dt` times
  // `input.audible` (smoothed in its bind, so it slows with a song's own fade), so that everything
  // turning of itself stands still in silence. Only while something is heard (above 0.3) does a
  // note or a hit count, so a stray onset in near silence moves nothing.
  state.phase += dt * input.audible;
  let heard = input.audible > 0.3;

  // The melody's notes come from the runtime (`input.note`, the strength of a note this frame,
  // 0 on none; never two within a tenth of a second); they are kept in turn for the emoji to
  // answer, while something is heard.
  state.note_rate *= exp(-dt / 2.0);
  if (input.note > 0.0 && heard) {
    state.note_count += 1.0;
    state.notes[u32(state.note_count) % 8u] = vec4f(t, max(input.note, 0.3), 0.0, 0.0);
    state.note_rate += 0.5;
  }

  // How many emoji the music calls for now: by the slow energy (`input.crowd_by_energy`, a
  // quarter to all: the section's, in its last four bars turning to the next one's, so a chorus
  // thickens as it comes); a quarter more while the melody is busy (four notes a second), a
  // quarter fewer while it rests. `crowd` says how far the number follows the music at all.
  let busy = clamp(state.note_rate / 4.0, 0.0, 1.0);
  let share = mix(1.0, input.crowd_by_energy * mix(0.75, 1.25, busy), params.crowd);

  // The hits, counted here. `input.hit` is a heard hit (at level A each beat followed; at B and C
  // every onset, which can be dense), so the count goes on by one only if at least half a beat
  // has passed since the last step and something is heard. From the count: the kick (rocking
  // the emoji, 0.25 s), the surge forward (0.3 s), and the crests' turn, the count with the last
  // step eased in over 0.15 s, so a crest steps once per counted step and settles.
  let beat_len = 60.0 / max(input.tempo, 20.0);
  state.step_age = min(state.step_age + dt, 1000.0);
  if (input.hit > 0.5 && heard && state.step_age >= 0.5 * beat_len) {
    state.count += 1.0;
    state.step_age = 0.0;
  }
  state.kick = exp(-state.step_age / 0.25);
  state.surge = exp(-state.step_age / 0.3);
  let arrived = clamp(state.step_age / 0.15, 0.0, 1.0);
  state.turn = state.count - 1.0 + (1.0 - exp(-3.0 * arrived)) / (1.0 - exp(-3.0));

  // Down the tunnel `per_beat` metres a beat of the tempo times how much is heard (`input.flight`;
  // the tempo is a calm one from how the music sounds when it has no beat), faster when the
  // energy is high (`energy_speed` says how much: 0 keeps it steady).
  let want = input.flight * (1.0 + params.energy_speed * (mix(0.6, 1.4, input.energy) - 1.0));
  state.speed += (want - state.speed) * (1.0 - exp(-dt / 0.6));
  state.s += state.speed * (1.0 + params.surge * state.surge) * dt;

  // The camera glides to the place the director cuts to (`shot.cuts` changes; shot `index` is the
  // place). The side changes only on the way to the middle place, by the shot's seed, so that no
  // glide swings across the tunnel. The glide takes `glide_bars` bars (bars of `input.meter` beats at
  // `input.tempo`, so it is a time even when there is no beat), but at most half of the shot, so
  // it never outlasts it. On a section's start (a cue of 0.5 or more) the badges coming take
  // another palette (`input.coming`, a pick on the cues). A place chosen by hand it glides to over
  // the same bars (at least two seconds, at most twenty), and the director's shots are ignored.
  let fixed = params.view > 0u;
  if (frame.first == 1u) {
    state.view = select(f32(shot.index), f32(params.view) - 1.0, fixed);
    state.side = select(-1.0, 1.0, select(shot.seed, random(3u), fixed) < 0.5);
    state.roll_to = select(shot.roll, 0.0, fixed);
    state.set_off = pose_of(u32(state.view), state.side, state.roll_to, fixed);
    state.last_view = t;
    state.glide = 1.0;
    state.cuts = shot.cuts;
  }
  let bar_time = input.meter * 60.0 / input.tempo;
  var go = -1.0;
  var glide = clamp(params.glide_bars * bar_time, 2.0, 20.0);
  if (fixed) {
    if (f32(params.view) - 1.0 != state.view) {
      go = f32(params.view) - 1.0;
    }
  } else if (shot.cuts != state.cuts) {
    go = f32(shot.index);
    if (go == 0.0 && shot.seed < 0.5) {
      state.side = -state.side;
    }
    state.roll_to = shot.roll;
    glide = params.glide_bars * bar_time;
    if (shot.length >= 0.0) {
      glide = min(glide, 0.5 * shot.length);
    }
    glide = max(glide, 0.25);
    // On a hard cut (`shot.hard`) it jumps there: inside a burst of cuts, the cut that ends the
    // burst, and the start of a high section, which the runtime dresses with a strong transition (a
    // hard cut is fine when a climax begins).
    if (shot.hard == 1u) {
      glide = 0.001;
    }
  }
  state.cuts = shot.cuts;
  if (go >= 0.0) {
    state.set_off = pose_now(t);
    state.view = go;
    state.last_view = t;
    state.glide = glide;
  }

  // Where it is, drifting a little round that, slowly: only a few degrees, over half a minute,
  // on the scene's own time, so it stands still in silence.
  let pose = pose_now(t);
  let ph = state.phase;
  let drift = params.drift * vec3f(0.03 * sin(ph * 0.21), 0.015 * sin(ph * 0.17 + 1.0), 0.04 * sin(ph * 0.13 + 2.0));
  let yaw = pose.turn.x + drift.x;
  let pitch = pose.turn.y + drift.y;
  let roll = pose.turn.z + pose.at.z + drift.z;
  state.cam_pos = vec3f(pose.at.x, pose.at.y, state.s);
  let fwd = vec3f(sin(yaw) * cos(pitch), sin(pitch), cos(yaw) * cos(pitch));
  let right0 = normalize(cross(vec3f(0.0, 1.0, 0.0), fwd));
  let up0 = cross(fwd, right0);
  state.cam_fwd = fwd;
  state.cam_right = right0 * cos(roll) + up0 * sin(roll);
  state.cam_up = up0 * cos(roll) - right0 * sin(roll);
  state.focal = params.focal * pose.turn.w;

  set_colours();

  // The SLOTS slots in sight: looking ahead, from a few behind (a turned camera sees beside
  // itself) on; looking back, the other way round; turned aside, as many either way (neither far
  // end is in the picture then). Each slot coming into sight far ahead takes the share the music
  // calls for now, and the palette.
  for (var k = 0u; k < TILES; k++) {
    state.tile_n[k] = 0u;
  }
  let here = floor(state.s / SLOT_LEN);
  let away = acos(clamp(fwd.z, -1.0, 1.0));
  let base = here - 4.0 - floor(f32(SLOTS - 8u) * smoothstep(1.1, 2.04, away));
  let farthest = here + f32(SLOTS) - 5.0;
  if (frame.first == 1u) {
    state.last_slot = here - f32(SLOTS) - 1.0;
  }
  for (var n = max(state.last_slot + 1.0, here - f32(SLOTS)); n <= farthest; n += 1.0) {
    state.slot_set[slot_cell(n)] = vec2f(share, input.coming);
  }
  state.last_slot = max(state.last_slot, farthest);
  for (var j = 0u; j < SLOTS; j++) {
    for (var q = 0u; q < PER; q++) {
      let i = j * PER + q;
      place(i, base + f32(j), q);
      project(i);
    }
  }
}
