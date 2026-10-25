// Once per frame: the beat clock, how hard they dance, the brows on an accent, the camera gliding
// between a few places, and every group's clip posed for this moment.

fn random(n: u32) -> f32 {
  return rv_unit(frame.seed * 2654435761u + n);
}

// ---- Dancing ----

// A pose: squashed (negative) or stretched, bent to the side, bowed forward, the wire twisted up
// its height, turned over backwards round its middle, the whole clip turned, and lifted off the
// floor.
struct Pose {
  squash: f32,
  sway: f32,
  bow: f32,
  twist: f32,
  flip: f32,
  spin: f32,
  hop: f32,
}

// A point of the clip (x across, y up, z towards the back) in the pose: squash and stretch about
// the foot, then bent as a rod bends, more the higher up, then turned and lifted.
fn posed(p: vec3f, pose: Pose) -> vec3f {
  var q = vec3f(p.x * (1.0 - 0.5 * pose.squash), p.y * (1.0 + pose.squash), p.z * (1.0 - 0.5 * pose.squash));
  q = vec3f(rv_rotate(q.xy, -pose.sway * q.y), q.z);
  let yz = rv_rotate(q.yz, -pose.bow * q.y);
  q = vec3f(q.x, yz.x, yz.y);
  let over = rv_rotate(vec2f(q.y - 0.5, q.z), pose.flip);
  q = vec3f(q.x, over.x + 0.5, over.y);
  let xz = rv_rotate(q.xz, pose.twist * q.y + pose.spin);
  q = vec3f(xz.x, q.y, xz.y);
  return q + vec3f(0.0, pose.hop + WIRE_R + 0.005, 0.0);
}

// The big move of bar n: 0 none (just the groove), 1 a bow, 2 a hop, 3 a whole turn, 4 a deep
// bend to one side, 5 a wiggle up the wire, 6 a backflip (only when it is loud; else a hop).
// Calm music keeps to the groove; the louder, the more often a move. The bar after a turn or a
// backflip may be a dizzy one instead.
fn move_of(n: f32) -> u32 {
  let h = hash(vec2f(n, state.section * 0.37 + 1.3));
  if (h > state.level * 1.1 - 0.15 || dizzy(n)) {
    return 0u;
  }
  let m = 1u + u32(hash(vec2f(n, state.section * 0.61 + 7.7)) * 6.0) % 6u;
  return select(m, 2u, m == 6u && state.level < params.flip_level);
}

// The move of bar n before any dizziness (so that whether bar n is dizzy can ask about n - 1).
fn spun(n: f32) -> bool {
  let h = hash(vec2f(n, state.section * 0.37 + 1.3));
  if (h > state.level * 1.1 - 0.15) {
    return false;
  }
  let m = 1u + u32(hash(vec2f(n, state.section * 0.61 + 7.7)) * 6.0) % 6u;
  return m == 3u || (m == 6u && state.level >= params.flip_level);
}

// Whether bar n is spent dizzy: now and then, after a whole turn or a backflip.
fn dizzy(n: f32) -> bool {
  return spun(n - 1.0) && hash(vec2f(n, state.section * 0.29 + 12.3)) < params.dizzy;
}

// The pose `late` beats behind the beat, danced `amp` hard.
fn pose_at(late: f32, amp: f32) -> Pose {
  var p: Pose;
  let b = state.clock - late;
  // The groove: down on every beat and back, swaying to one side on one beat and the other on
  // the next, a little nod.
  let hit = exp(-fract(b) * 6.0) * state.sure;
  p.squash = -0.16 * amp * hit + 0.04 * amp * sin(fract(b) * 3.14159) * state.sure;
  p.sway = 0.28 * amp * cos(3.14159 * b);
  p.bow = 0.1 * amp * hit;
  // The bar's big move.
  let bar = b / state.per_bar + state.bar_shift;
  let n = floor(bar);
  let u = fract(bar);
  // Dizzy, the music forgotten: the head wanders round in slow circles, the clip unsteady on its
  // foot.
  if (dizzy(n)) {
    let t = (frame.time - late * state.beat_len) * 4.4;
    let fade = smoothstep(0.0, 0.1, u) * (1.0 - smoothstep(0.85, 1.0, u));
    p.squash = -0.06 * fade * (0.5 + 0.5 * sin(t * 1.3));
    p.sway = mix(p.sway, 0.38 * sin(t), fade);
    p.bow = mix(p.bow, 0.32 * cos(t), fade);
  }
  let half = sin(3.14159 * clamp(u * 2.0, 0.0, 1.0));
  let side = select(-1.0, 1.0, hash(vec2f(n, 3.3)) < 0.5);
  // Turning to face the camera: a quick turn that swings a little past and back, with a small
  // hop, the chorus following in its wave.
  let turn = clamp((bar - state.face_bar) * state.per_bar / max(params.turn_beats, 0.25), 0.0, 1.0);
  let s1 = turn - 1.0;
  let swing = 1.0 + 2.70158 * s1 * s1 * s1 + 1.70158 * s1 * s1;
  p.spin = mix(state.face_from, state.face_to, swing);
  p.hop += 0.12 * amp * sin(3.14159 * turn);
  switch move_of(n) {
    case 1u: {
      p.bow += 0.9 * amp * half;
    }
    case 2u: {
      let x = clamp(u * state.per_bar, 0.0, 1.0);
      p.hop += 0.45 * amp * 4.0 * x * (1.0 - x);
      p.squash += 0.12 * amp * 4.0 * x * (1.0 - x);
    }
    case 3u: {
      p.spin += 6.28318 * side * smoothstep(0.0, 0.75, u);
    }
    case 4u: {
      p.sway += 1.0 * amp * side * half;
    }
    case 5u: {
      p.twist += 1.6 * amp * sin(6.28318 * u * 2.0) * (1.0 - u);
    }
    case 6u: {
      // Crouch, spring up turning over backwards, land and give.
      let crouch = smoothstep(0.0, 0.12, u) * (1.0 - smoothstep(0.12, 0.18, u));
      let x = clamp((u - 0.15) / 0.6, 0.0, 1.0);
      let land = smoothstep(0.74, 0.78, u) * (1.0 - smoothstep(0.78, 0.95, u));
      p.squash += -0.25 * crouch - 0.2 * land + 0.1 * sin(3.14159 * x);
      p.hop += (0.4 + 0.6 * amp) * 4.0 * x * (1.0 - x);
      p.flip = 6.28318 * smoothstep(0.0, 1.0, x);
    }
    default: {}
  }
  return p;
}

// The face a dancer `late` beats behind wants, and whether the dance requires it (1) or it is
// only the mood (0): 0 eyes open, 1 >_< (the peak of the song), 2 ^ ^ (happy, in a big move),
// 3 @ @ (dizzy, the bar after a whole turn), 4 wide (an accent), 5 – – (drowsy, when it is
// quiet), 6 stars, 7 a blink.
fn face_at(late: f32, seed: f32) -> vec2f {
  if (params.face_lock > 0.5) {
    return vec2f(params.face_lock - 1.0, 1.0);
  }
  let b = state.clock - late;
  let bar = b / state.per_bar + state.bar_shift;
  let n = floor(bar);
  // The mood reaches the chorus in the wave, as the dance does.
  let mood = select(state.mood, state.mood_prev, bar < state.mood_bar);
  // Eyes shut while turning round (open eyes that stay on the camera as the body turns look
  // eerie): turning to the camera, or the whole turn of a bar's big move. A mood with the eyes
  // shut or half shut stays.
  let turn = (bar - state.face_bar) * state.per_bar / max(params.turn_beats, 0.25);
  if ((turn > -0.15 && turn < 1.1) || (move_of(n) == 3u && fract(bar) < 0.8)) {
    return vec2f(select(2.0, mood, mood == 1.0 || mood == 2.0 || mood == 5.0), 1.0);
  }
  // >_< from the crouch to the landing of a backflip.
  if (move_of(n) == 6u && fract(bar) < 0.92) {
    return vec2f(1.0, 1.0);
  }
  if (dizzy(n)) {
    return vec2f(3.0, 1.0);
  }
  // Wide eyed for a moment as a section lifts the energy.
  if (state.surprise > 0.5 && late < 0.5) {
    return vec2f(4.0, 0.0);
  }
  // Otherwise the mood; open eyes blink about every seven seconds, the gaps uneven.
  let blink = frame.time / 7.0 + seed;
  if (mood == 0.0 && fract(blink) < 0.02 && hash(vec2f(floor(blink), seed)) < 0.8) {
    return vec2f(7.0, 0.0);
  }
  return vec2f(mood, 0.0);
}

// How long a face of the mood is kept at least before another mood face (a surprise, the next
// mood, a blink) may take its place, and how long the lids close between an open face and
// another.
const HOLD: f32 = 1.2;
const BLINK_THROUGH: f32 = 0.07;

// The face group g shows, from the one it wants: what the dance requires comes at once, the
// mood's faces wait until the face before has been held a while (so faces do not flick from one
// to the next), and the lids close for a moment between an open face and another.
fn show_face(g: u32, want: vec2f) -> f32 {
  var f = state.faces[g];
  if (want.x != f.x) {
    let firm = want.y > 0.5 || f.w > 0.5;
    if (firm || f.x == 7.0 || frame.time - f.y > HOLD) {
      f = vec4f(want.x, frame.time, f.x, want.y);
      state.faces[g] = f;
    }
  }
  let open_now = f.x == 0.0 || f.x == 3.0 || f.x == 4.0 || f.x == 6.0;
  let open_before = f.z == 0.0 || f.z == 3.0 || f.z == 4.0 || f.z == 6.0;
  let through = (open_now || open_before) && f.x != 7.0 && f.z != 7.0;
  return select(f.x, 7.0, through && frame.time - f.y < BLINK_THROUGH && params.face_lock < 0.5);
}

// The mood, by how hard they dance: >_< (now and then stars) when it is loud, ^ ^ or open eyes in
// between, open or drowsy when it is quiet. `pick` chooses between the two; when `keep` is set, a
// mood that still fits stays. Faces that change by the bar look restless.
fn choose_mood(level: f32, pick: f32, keep: bool) {
  var a = 0.0;
  var b = 0.0;
  var share = 0.5;
  if (level > params.peak) {
    a = 1.0;
    b = 6.0;
    share = 0.2;
  } else if (level > 0.45) {
    b = 2.0;
  } else if (level < 0.3) {
    b = 5.0;
  }
  var mood = select(a, b, pick < share);
  if (keep && (state.mood == a || state.mood == b)) {
    mood = state.mood;
  }
  if (mood != state.mood) {
    state.mood_prev = state.mood;
    state.mood = mood;
    state.mood_bar = state.clock / state.per_bar + state.bar_shift;
  }
}

// Write group g's clip in the pose: the wire, the eyes and the brows, and the sphere that holds it
// all. Gives how far out from its foot it reaches over the floor.
fn write_group(g: u32, pose: Pose, late: f32) -> f32 {
  let base = g * POINTS;
  var lo = vec3f(1e9);
  var hi = vec3f(-1e9);
  for (var i = 0u; i < WIRE; i++) {
    let q = posed(vec3f(PATH[i], 0.0), pose);
    state.pts[base + i] = vec4f(q, 0.0);
    lo = min(lo, q - WIRE_R);
    hi = max(hi, q + WIRE_R);
  }
  // The eyes sit in front of the wire near the top, looking about a little; the brows above them,
  // lifted on an accent.
  let t = frame.time + f32(g) * 0.7;
  let look = vec2f(0.35 * sin(t * 0.43) * sin(t * 0.19), 0.15 * sin(t * 0.31));
  let face = show_face(g, face_at(late, f32(g) * 0.13));
  // Which way the head faces and which way is up on it, for the face drawn on the eyes.
  let head = posed(vec3f(0.0, 0.7, -0.06), pose);
  let fwd = normalize(posed(vec3f(0.0, 0.7, -0.07), pose) - head);
  let up = normalize(posed(vec3f(0.0, 0.71, -0.06), pose) - head);
  state.pts[base].w = fwd.x;
  state.pts[base + 1u].w = fwd.y;
  state.pts[base + 2u].w = fwd.z;
  state.pts[base + 3u].w = up.x;
  state.pts[base + 4u].w = up.y;
  state.pts[base + 5u].w = up.z;
  let lift =0.05 * state.brow + 0.015 * exp(-fract(state.clock - late) * 6.0) * state.sure;
  for (var e = 0u; e < 2u; e++) {
    let s = select(-1.0, 1.0, e == 1u);
    let eye = posed(vec3f(0.08 * s, 0.7, -0.06), pose);
    state.pts[base + WIRE + e] = vec4f(eye, select(look.x, look.y, e == 1u));
    let a = posed(vec3f(0.08 * s - 0.05, 0.83 + lift, -0.07), pose);
    let c = posed(vec3f(0.08 * s + 0.05, 0.83 + lift + 0.025 * s * (1.0 - 2.0 * state.brow), -0.07), pose);
    state.pts[base + WIRE + 2u + e * 2u] = vec4f(a, face);
    state.pts[base + WIRE + 3u + e * 2u] = vec4f(c, 0.0);
    lo = min(lo, eye - EYE_R);
    hi = max(hi, eye + EYE_R);
  }
  let mid = 0.5 * (lo + hi);
  state.bound[g] = vec4f(mid, length(hi - lo) * 0.5 + 0.02);
  return max(max(-lo.x, hi.x), max(-lo.z, hi.z));
}

// Pose group g. A pixel looks for a chorus dancer only in the floor cell under it, so a dancer of
// the chorus must stay within its own cell: one that would lean out of it dances that bar a
// little smaller instead. (The lead is looked for everywhere and dances in full.)
fn pose_group(g: u32) {
  let late = select(f32(g) * 0.125, 0.0, g == 0u);
  var amp = state.level * select(params.chorus, 1.0, g == 0u);
  let room = 0.5 * CELL - 0.02;
  for (var k = 0; k < 3; k++) {
    let reach = write_group(g, pose_at(late, amp), late);
    if (g == 0u || reach <= room) {
      break;
    }
    amp *= room / reach * 0.97;
  }
}

// ---- Camera ----

// How far off (radians) the camera must have gone round the dancers before they turn to face it.
const TURN: f32 = 1.0;

// A place to film from: where the camera is round the lead (bearing, distance out over the
// floor, height), where it looks and the lens.
struct Shot {
  pol: vec3f,
  aim: vec3f,
  lens: f32,
}

// View v filmed from one side of the lead or the other, `s` (0 to 1) through its own slow move:
// 0 in front of the lead, the chorus behind, pushing in; 1 low and to one side, looking up at the
// lead, edging round; 2 high and far, the whole field of dancers, craning down; 3 close beside the
// lead, edging round; 4 above the chorus looking back to the lead, sinking; 5 circling the lead.
fn shot(v: f32, side: f32, s: f32) -> Shot {
  var a = vec3f(0.0);
  var b = vec3f(0.0);
  var sh: Shot;
  sh.aim = vec3f(0.0, 0.6, 0.0);
  sh.lens = 0.42;
  if (v < 0.5) {
    a = vec3f(-0.12 * side, 3.3, 0.75);
    b = vec3f(0.12 * side, 2.7, 0.75);
  } else if (v < 1.5) {
    a = vec3f(0.45 * side, 2.7, 0.25);
    b = vec3f(0.75 * side, 2.5, 0.3);
    sh.aim = vec3f(0.0, 0.75, 0.6);
    sh.lens = 0.5;
  } else if (v < 2.5) {
    a = vec3f(0.35 * side, 5.6, 4.6);
    b = vec3f(0.15 * side, 4.9, 3.9);
    sh.aim = vec3f(0.0, 0.2, 4.0);
    sh.lens = 0.5;
  } else if (v < 3.5) {
    a = vec3f(0.5 * side, 1.8, 0.85);
    b = vec3f(0.85 * side, 1.6, 0.85);
    sh.lens = 0.36;
  } else if (v < 4.5) {
    a = vec3f((3.14159 - 0.38) * side, 8.6, 3.2);
    b = vec3f((3.14159 - 0.22) * side, 8.0, 2.7);
    sh.aim = vec3f(0.0, 0.4, 0.0);
    sh.lens = 0.45;
  } else {
    // Craning up over the chorus as it passes behind the lead.
    a = vec3f(state.ring, 2.8, 0.9 + 1.6 * (0.5 - 0.5 * cos(state.ring)));
    b = a;
  }
  sh.pol = mix(a, b, 0.5 + (s - 0.5) * params.drift);
  return sh;
}

// Go to another view, gliding there from where the camera is over `len` seconds. With a section
// label the view is the label's (sections that sound alike are filmed alike); calm music is filmed
// from further off, loud music from close and circling.
fn choose_view(label: f32, energy: f32, len: f32) {
  let styled = label >= 0.0;
  let r1 = select(random(1u), fract((label + 1.0) * 0.6180339 + 0.137), styled);
  let r2 = select(random(2u), fract((label + 1.0) * 0.7548777 + 0.291), styled);
  var views = array<f32, 4>(0.0, 2.0, 4.0, 1.0);
  if (energy > 0.6) {
    views = array<f32, 4>(3.0, 5.0, 1.0, 0.0);
  }
  var i = u32(r1 * 4.0) % 4u;
  if (!styled && views[i] == state.view) {
    i = (i + 1u) % 4u;
  }
  var v = views[i];
  if (params.view_lock > 0.5) {
    v = params.view_lock - 1.0;
  }
  state.view = v;
  state.side = select(-1.0, 1.0, r2 < 0.5);
  // The circling view starts where the camera is.
  state.ring = state.pol.x;
  state.pol_from = state.pol;
  state.aim_from = state.aim_base;
  state.lens_from = state.lens;
  state.glide = 0.0;
  state.glide_len = len;
  state.shot = 0.0;
  state.last_view = frame.time;
}

fn update() {
  let dt = frame.dt;
  let t = frame.time;
  if (frame.first == 1u) {
    state.beat_len = 0.5;
    state.per_bar = 4.0;
    state.level = 0.5;
    state.planned = -1.0;
    choose_view(-1.0, 0.5, 1.0);
    state.glide = 1.0;
  }

  // The beat clock: it runs at the tempo and is set to the beat when one comes (never back).
  let sure = music.bpm > 0.0 && music.beat_confidence > params.conf;
  let beat = music.beats > 0u && sure;
  state.sure += (f32(sure) - state.sure) * (1.0 - exp(-dt / 0.5));
  state.beat_len = select(0.5, 60.0 / max(music.bpm, 1.0), sure);
  state.clock += dt / state.beat_len;
  if (beat) {
    state.clock = max(ceil(state.clock - 0.35), state.clock);
  }
  state.per_bar = select(4.0, clamp(music.beats_per_bar, 2.0, 7.0), music.has_chart == 1u && music.beats_per_bar > 0.0);
  // Bars start where the music's do: on a downbeat the bar position is set to a whole number.
  if (music.bars > 0u) {
    let at = state.clock / state.per_bar + state.bar_shift;
    state.bar_shift += round(at) - at;
  }
  let bar_len = state.per_bar * state.beat_len;
  let now_bar = state.clock / state.per_bar + state.bar_shift;

  // How hard they dance: the section's energy with a chart, else the loudness.
  let chart = music.has_chart == 1u;
  let goal = select(clamp(music.loudness, 0.0, 1.0), music.section_energy, chart);
  state.energy += (goal - state.energy) * (1.0 - exp(-dt / select(1.5, 0.5, chart)));
  state.level = clamp(0.25 + 0.85 * state.energy, 0.2, 1.0) * params.dance;

  // Accents lift the brows: a section that lifts the energy, or a hit far above the usual.
  var accent = false;
  let new_section = chart && music.sections > 0u;
  if (new_section) {
    accent = music.section_energy > state.last_energy + 0.1;
    state.last_energy = music.section_energy;
    state.section = music.section;
  }
  if (music.low > 0.95 && state.energy > 0.6 && t - state.last_accent > 6.0) {
    accent = true;
  }
  if (accent) {
    state.last_accent = t;
    state.brow = 1.0;
  }
  state.brow *= exp(-dt / 0.6);
  // Only a section that lifts the energy widens the eyes; a loud hit just lifts the brows.
  if (new_section && accent) {
    state.surprise = 1.0;
  }
  state.surprise *= exp(-dt / 0.6);

  // The mood: with a chart, a section's (the same for sections that sound alike); without, a
  // phrase's, kept while it still fits how hard they dance.
  let section_level = clamp(0.25 + 0.85 * music.section_energy, 0.2, 1.0) * params.dance;
  let phrase = floor(now_bar / max(params.mood_bars, 1.0));
  if (frame.first == 1u) {
    state.mood = -1.0;
    choose_mood(state.level, 1.0, false);
    state.mood_prev = state.mood;
  } else if (new_section) {
    choose_mood(section_level, fract((music.section_label + 1.0) * 0.5698403 + 0.41), false);
  } else if (!chart && phrase != state.mood_phrase) {
    choose_mood(state.level, hash(vec2f(phrase, 2.1)), hash(vec2f(phrase, 5.3)) < 0.65);
  }
  state.mood_phrase = phrase;

  // The camera: to another view for a section (gliding there over the bars before it, so as to
  // arrive as it starts), or every few bars, or now and then without a beat.
  let glide_len = params.glide * bar_len;
  let ahead = chart && music.next_label != music.section_label && music.section_left > 0.0
    && music.section_left < glide_len && music.section_left > 0.5 * glide_len
    && state.planned != music.section + 1.0;
  if (ahead && t - state.last_view > 4.0) {
    choose_view(music.next_label, music.next_energy, music.section_left);
    state.planned = music.section + 1.0;
  } else if (new_section && music.section != state.planned && t - state.last_view > 4.0) {
    choose_view(music.section_label, music.section_energy, glide_len);
    state.planned = music.section;
  } else if (music.bars > 0u && t - state.last_view > params.view_gap
    && u32(round(now_bar)) % max(u32(params.view_bars), 1u) == 0u
    && !(chart && music.section_left < 2.0 * glide_len)) {
    state.section += 101.0;
    choose_view(-1.0, state.energy, glide_len);
  } else if (music.bpm <= 0.0 && t - state.last_view > 30.0) {
    choose_view(-1.0, state.energy, glide_len);
  }
  // The circling view goes round once in so many bars, the way of its side.
  if (state.view > 4.5) {
    state.ring += state.side * dt * 6.28318 / max(params.ring_bars * bar_len, 1.0);
  }
  // Each view moves slowly through the view's time, slowing to a stop; a glide goes round the
  // lead (bearing, distance and height each in turn), never through the dancers.
  state.glide = min(state.glide + dt / max(state.glide_len, 0.1), 1.0);
  state.shot = min(state.shot + dt / max(params.view_bars * bar_len, 1.0), 1.0);
  let s = 1.0 - (1.0 - state.shot) * (1.0 - state.shot);
  let to = shot(state.view, state.side, s);
  let k = smoothstep(0.0, 1.0, state.glide);
  let round_by = atan2(sin(to.pol.x - state.pol_from.x), cos(to.pol.x - state.pol_from.x));
  state.pol = vec3f(state.pol_from.x + round_by * k, mix(state.pol_from.yz, to.pol.yz, k));
  state.cam = vec3f(sin(state.pol.x) * state.pol.y, state.pol.z, -cos(state.pol.x) * state.pol.y);
  state.aim_base = mix(state.aim_from, to.aim, k);
  state.lens = mix(state.lens_from, to.lens, k);
  // The camera looks up after the lead when it hops or flips, a little late.
  state.lift += (pose_at(0.0, state.level).hop - state.lift) * (1.0 - exp(-dt / 0.15));
  state.aim = state.aim_base + vec3f(0.0, 0.6 * state.lift, 0.0);

  // The dancers turn to face the camera when it has gone well round them, on a bar line (without
  // a beat, now and then), once the last turn has spread through the chorus. On the circling view
  // they turn a little past the camera, the way it goes, so as not to turn again too soon.
  let off = atan2(sin(state.pol.x - state.face_to), cos(state.pol.x - state.face_to));
  let settled = (now_bar - state.face_bar) * state.per_bar > params.turn_beats + 1.5;
  let was_bar = (state.clock - dt / state.beat_len) / state.per_bar + state.bar_shift;
  let due = floor(now_bar) != floor(was_bar);
  if (abs(off) > TURN && settled && due) {
    state.face_from = state.face_to;
    state.face_to += off + select(0.0, 0.6 * state.side, state.view > 4.5);
    state.face_bar = round(now_bar);
  }

  // The paper: pale yellow, or in the player's colour.
  var paper = params.paper.rgb;
  if (params.player_colour == 1u && frame.accent.a > 0.5) {
    let lab = rv_linear_to_oklab(frame.accent.rgb);
    let h = atan2(lab.z, lab.y);
    paper = max(rv_oklab_to_linear(vec3f(0.86, 0.07 * cos(h), 0.07 * sin(h))), vec3f(0.0));
  }
  state.paper = paper;

  for (var g = 0u; g < GROUPS; g++) {
    pose_group(g);
  }

  post.exposure = params.exposure;
}
