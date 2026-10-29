// Once per frame: how hard they dance, the brows on an impact or a lift, the camera gliding
// between a few places as the director cuts (`shot`), and every group's clip posed for this
// moment. The beat clock, the bar length and how far the beat is followed are the runtime's
// (`input.clock`, `input.meter`, `input.locked`).
//
// In silence they stand still and the camera stops (as Growth and Torii Sea):
// the dance's strength, the brows' bob, the dizzy wander, the view's slow move and the circling
// view all go by `input.audible`, and a bar's big move does not start (`input.moves`). The eyes'
// wandering and the blinks go on: they are alive, not dancing.

// Seconds per beat, from the tempo the runtime gives (calm and feel-based when no beat is known).
fn beat_len() -> f32 {
  return 60.0 / input.tempo;
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

// What bar n does, from the record made as it began (see `decide_bar`); a bar with no record
// (before the first, or after a jump in time) has no move and is not dizzy.
fn rec_slot(n: f32) -> i32 {
  let slot = u32(max(n, 0.0)) % 4u;
  return select(-1, i32(slot), state.rec_bar[slot] == n);
}

// The big move of bar n: 0 none (just the groove), 1 a bow, 2 a hop, 3 a whole turn, 4 a deep
// bend to one side, 5 a wiggle up the wire, 6 a backflip.
fn move_of(n: f32) -> u32 {
  let s = rec_slot(n);
  return select(0u, u32(state.rec_move[max(s, 0)]), s >= 0);
}

// Which way bar n's move goes (-1 or 1).
fn side_of(n: f32) -> f32 {
  let s = rec_slot(n);
  return select(1.0, state.rec_side[max(s, 0)], s >= 0);
}

// The amplitude bar n's move is danced at (the dance's strength as the bar began, before what is
// heard: a move under way still calms with a fade and stills in silence, see pose_at).
fn move_amp_of(n: f32) -> f32 {
  let s = rec_slot(n);
  return select(0.0, state.rec_amp[max(s, 0)], s >= 0);
}

// Whether bar n is spent dizzy: now and then, the bar after a whole turn or a backflip.
fn dizzy(n: f32) -> bool {
  let s = rec_slot(n);
  return s >= 0 && state.rec_dizzy[s] > 0.5;
}

// Decide bar n, as it begins on the lead, once: its move, side, dizziness and amplitude go in the
// record and stay. Calm music keeps to the groove; the louder the section (`level`) the more likely
// a move in a bar that may have one, and a backflip only when it is loud (`flip_level`), both read
// here and never again. A big move needs enough heard (`moves`), and room: after one at least
// `move_every` bars to the next, one more after a whole turn or a backflip (the moves are
// interrupted by new ones, and do not run on). The bar after a turn or a backflip may be dizzy
// instead, and that bar is one of the bars of room.
fn decide_bar(n: f32) {
  let slot = u32(max(n, 0.0)) % 4u;
  var mv = 0u;
  var dz = 0.0;
  let side = select(-1.0, 1.0, hash(vec2f(n, 3.3)) < 0.5);
  if (input.moves > 0.5) {
    // What the bar before did (a bar before the record's four, or never seen, did nothing).
    let before = move_of(n - 1.0);
    var since = n - state.move_bar;
    if (since < 0.0) {
      since = 1e9;
    }
    if ((before == 3u || before == 6u) && hash(vec2f(n, state.section * 0.29 + 12.3)) < params.dizzy) {
      dz = 1.0;
    } else if (since >= state.move_gap
        && hash(vec2f(n, state.section * 0.37 + 1.3)) <= state.level * 1.1 - 0.15) {
      mv = 1u + u32(hash(vec2f(n, state.section * 0.61 + 7.7)) * 6.0) % 6u;
      if (mv == 6u && state.level < params.flip_level) {
        mv = 2u;
      }
    }
  }
  if (mv > 0u) {
    state.move_bar = n;
    state.move_gap = max(round(params.move_every), 1.0) + select(0.0, 1.0, mv == 3u || mv == 6u);
  }
  state.rec_bar[slot] = n;
  state.rec_move[slot] = f32(mv);
  state.rec_side[slot] = side;
  state.rec_dizzy[slot] = dz;
  state.rec_amp[slot] = strength();
}

// How hard they dance, before what is heard: the dance's strength and the energy of the moment
// (spread over its range, as Torii Sea's spirits).
fn strength() -> f32 {
  return params.dance * mix(1.0, mix(0.5, 1.25, input.energy), params.energy_dance);
}

// Smootherstep of x clamped to 0 to 1: no speed at either end.
fn ease(x: f32) -> f32 {
  let t = clamp(x, 0.0, 1.0);
  return t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
}

// A move's envelope over its bar (u, 0 to 1): rising over the first beat and settling over the
// last, so it starts and ends at rest.
fn envelope(u: f32) -> f32 {
  let w = clamp(1.0 / input.meter, 0.05, 0.5);
  return ease(u / w) * ease((1.0 - u) / w);
}

// The dip of a hit, `a` beats after it: down over a tenth of a beat to the bottom (a hit cannot be
// seen coming), then back up as the groove always did. It is 0 at the hit itself, so it never drops
// in one frame (no per-beat jolt).
fn dip_at(a: f32) -> f32 {
  return select(exp(-(a - 0.1) * 6.0), ease(a / 0.1), a < 0.1);
}

// The dip on the beat clock, at phase b: the beat is known before it comes, so the dip goes down
// over the last 0.15 of the beat before and is at its bottom on the beat, then back up.
fn dip_on_beat(b: f32) -> f32 {
  let a = fract(b + 0.15) - 0.15;
  return select(exp(-a * 6.0), ease((a + 0.15) / 0.15), a < 0.0);
}

// The groove of the dancer `late` beats behind: the dip (easing down to its bottom, on the beat
// on the clock and a tenth of a beat after a hit, then up again), the nod (a swell over the beat) and the sway (-1 to 1, one
// side or the other).
// With the beat followed (`locked`, A) it is locked to the beat clock: down on every beat, the
// side changing on each. Where it is not (B and C) it is on the hits instead, the heard onsets: the
// dip and the nod come on each hit and the sway swings, eased over half a beat, to the other side
// on each, a hit within half a beat of the one before not counting (so dense onsets do not shake
// them). It is later for a dancer further back in the wave as on the clock, and an upright
// stance returns after a couple of beats with no hit. Between the two it is the mix, as `locked`
// fades.
fn groove_at(late: f32) -> vec3f {
  let b = input.clock - late;
  let on_beat = vec3f(dip_on_beat(b), sin(fract(b) * 3.14159), cos(3.14159 * b));
  if (input.locked >= 1.0) {
    return on_beat;
  }
  let bl = beat_len();
  let t = frame.time - late * bl;
  var on_hits = vec3f(0.0);
  for (var m = 0u; m < min(state.hit_n, 8u); m++) {
    let slot = (state.hit_n + 7u - m) % 8u;
    let ht = state.hit_t[slot / 4u][slot % 4u];
    if (ht <= t) {
      let age = (t - ht) / bl;
      // The side the swing leaves: hit k goes from one side to the other, alternately.
      let from_side = select(1.0, -1.0, (state.hit_n - m - 1u) % 2u == 1u);
      let swing = from_side * cos(3.14159 * clamp(age * 2.0, 0.0, 1.0));
      on_hits = vec3f(dip_at(age), sin(3.14159 * clamp(age, 0.0, 1.0)), swing * (1.0 - smoothstep(2.0, 4.0, age)));
      break;
    }
  }
  return mix(on_hits, on_beat, input.locked);
}

// The pose `late` beats behind the beat; `scale` is how much of the dance this dancer gets (the
// lead 1, the chorus less), which multiplies the dance's strength now (the groove) and the
// strength of the bar's own record (its big move).
fn pose_at(late: f32, scale: f32) -> Pose {
  var p: Pose;
  let amp = state.amp * scale;
  let b = input.clock - late;
  // The bar's big move, from the record made as the bar began (the runtime aligns the clock so
  // that a whole bar starts on the music's downbeat).
  let bar = b / input.meter;
  let n = floor(bar);
  let u = fract(bar);
  let mv = move_of(n);
  let mamp = move_amp_of(n) * scale * input.audible;
  // The groove: down on every beat and back, swaying to one side on one beat and the other on
  // the next, a little nod. While a big move is under way it gives way to it, fading down as the
  // move eases in and back up as it eases out, so the two are not simply added.
  let groove = groove_at(late);
  let own = envelope(u) * select(0.0, 1.0, mv > 0u);
  p.squash = (-0.16 * amp * groove.x + 0.04 * amp * groove.y) * (1.0 - 0.7 * own);
  p.sway = 0.28 * amp * groove.z * (1.0 - own);
  p.bow = 0.1 * amp * groove.x * (1.0 - 0.7 * own);
  // Dizzy, the music forgotten: the head wanders round in slow circles, the clip unsteady on its
  // foot (and, a dance like the rest, stilled in silence).
  if (dizzy(n)) {
    let t = (frame.time - late * beat_len()) * 4.4;
    let fade = smoothstep(0.0, 0.1, u) * (1.0 - smoothstep(0.85, 1.0, u));
    p.squash = -0.06 * fade * (0.5 + 0.5 * sin(t * 1.3)) * input.audible;
    p.sway = mix(p.sway, 0.38 * sin(t) * input.audible, fade);
    p.bow = mix(p.bow, 0.32 * cos(t) * input.audible, fade);
  }
  // Down, held a little, and back, over the whole bar: eased at both ends.
  let fill = ease(u / 0.3) * ease((1.0 - u) / 0.3);
  let side = side_of(n);
  // Turning to face the camera: a quick turn that swings a little past and back, with a small
  // hop, the chorus following in its wave.
  let turn = clamp((bar - state.face_bar) * input.meter / max(params.turn_beats, 0.25), 0.0, 1.0);
  let s1 = turn - 1.0;
  let swing = 1.0 + 2.70158 * s1 * s1 * s1 + 1.70158 * s1 * s1;
  p.spin = mix(state.face_from, state.face_to, swing);
  p.hop += 0.12 * amp * sin(3.14159 * turn);
  switch mv {
    case 1u: {
      p.bow += 0.9 * mamp * fill;
    }
    case 2u: {
      // Crouch, spring up (a beat and a half in the air), land with a give that settles.
      let crouch = ease(u / 0.1) * (1.0 - ease((u - 0.1) / 0.05));
      let air = min(1.5 / input.meter, 0.6);
      let x = clamp((u - 0.15) / air, 0.0, 1.0);
      let land = ease((u - 0.15 - air) / 0.03) * (1.0 - ease((u - 0.18 - air) / 0.2));
      p.hop += 0.45 * mamp * 4.0 * x * (1.0 - x);
      p.squash += -0.15 * mamp * crouch - 0.15 * mamp * land + 0.12 * mamp * sin(3.14159 * x);
    }
    case 3u: {
      p.spin += 6.28318 * side * smoothstep(0.0, 0.75, u);
    }
    case 4u: {
      p.sway += 1.0 * mamp * side * fill;
    }
    case 5u: {
      // The wire wriggles up from rest and dies away to rest.
      p.twist += 1.6 * mamp * sin(6.28318 * u * 2.0) * ease(u / 0.12) * (1.0 - ease((u - 0.2) / 0.8));
    }
    case 6u: {
      // Crouch, spring up turning over backwards, land and give.
      let crouch = smoothstep(0.0, 0.12, u) * (1.0 - smoothstep(0.12, 0.18, u));
      let x = clamp((u - 0.15) / 0.6, 0.0, 1.0);
      let land = smoothstep(0.74, 0.78, u) * (1.0 - smoothstep(0.78, 0.97, u));
      p.squash += -0.25 * crouch - 0.2 * land + 0.1 * sin(3.14159 * x);
      p.hop += (0.4 + 0.6 * mamp) * 4.0 * x * (1.0 - x);
      p.flip = 6.28318 * smoothstep(0.0, 1.0, x);
    }
    default: {}
  }
  return p;
}

// The face a dancer `late` beats behind wants, and whether the dance requires it (1) or it is
// only the mood (0): 0 eyes open, 1 >_< (the peak of the song), 2 ^ ^ (happy, in a big move),
// 3 @ @ (dizzy, the bar after a whole turn), 4 wide (a section that lifts the energy), 5 – – (drowsy, when it is
// quiet), 6 stars, 7 a blink.
fn face_at(late: f32, seed: f32) -> vec2f {
  if (params.face_lock > 0.5) {
    return vec2f(params.face_lock - 1.0, 1.0);
  }
  let b = input.clock - late;
  let bar = b / input.meter;
  let n = floor(bar);
  // The mood reaches the chorus in the wave, as the dance does.
  let mood = select(state.mood, state.mood_prev, bar < state.mood_bar);
  // Eyes shut while turning round (open eyes that stay on the camera as the body turns look
  // eerie): turning to the camera, or the whole turn of a bar's big move. A mood with the eyes
  // shut or half shut stays.
  let turn = (bar - state.face_bar) * input.meter / max(params.turn_beats, 0.25);
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
    state.mood_bar = input.clock / input.meter;
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
  // lifted on an impact or a lift of the energy.
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
  // (The small bob goes with the dip, on the beat or on the hits, and stills in silence.)
  let lift = 0.05 * state.brow + 0.015 * groove_at(late).x * input.audible;
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
  var scale = select(params.chorus, 1.0, g == 0u);
  let room = 0.5 * CELL - 0.02;
  for (var k = 0; k < 3; k++) {
    let reach = write_group(g, pose_at(late, scale), late);
    if (g == 0u || reach <= room) {
      break;
    }
    scale *= room / reach * 0.97;
  }
}

// ---- Camera ----

// How far off (radians) the camera must have gone round the dancers before they turn to face it.
const TURN: f32 = 1.0;

// A place to film from: where the camera is round the lead (bearing, distance out over the
// floor, height), where it looks and the lens.
struct ViewShot {
  pol: vec3f,
  aim: vec3f,
  lens: f32,
}

// View v filmed from one side of the lead or the other, `s` (0 to 1) through its own slow move:
// 0 in front of the lead, the chorus behind, pushing in; 1 low and to one side, looking up at the
// lead, edging round; 2 high and far, the whole field of dancers, craning down; 3 close beside the
// lead, edging round; 4 above the chorus looking back to the lead, sinking; 5 circling the lead.
// The burst's four, each held (a hard cut lasts a beat) and each from a new place, nearer and then
// from above: 6 medium in front of the lead; 7 close to its face from one side; 8 its eyes and brows
// from the other side through a long lens; 9 high above it and the field.
fn view_shot(v: f32, side: f32, s: f32) -> ViewShot {
  var a = vec3f(0.0);
  var b = vec3f(0.0);
  var sh: ViewShot;
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
  } else if (v < 5.5) {
    // Craning up over the chorus as it passes behind the lead.
    a = vec3f(state.ring, 2.8, 0.9 + 1.6 * (0.5 - 0.5 * cos(state.ring)));
    b = a;
  } else if (v < 6.5) {
    a = vec3f(0.18 * side, 2.0, 0.65);
    b = a;
    sh.aim = vec3f(0.0, 0.55, 0.4);
    sh.lens = 0.4;
  } else if (v < 7.5) {
    a = vec3f(1.2 * side, 1.4, 0.8);
    b = a;
    sh.aim = vec3f(0.0, 0.72, 0.0);
    sh.lens = 0.42;
  } else if (v < 8.5) {
    a = vec3f(0.5 * side, 2.7, 0.8);
    b = a;
    sh.aim = vec3f(0.0, 0.76, 0.0);
    sh.lens = 0.21;
  } else {
    a = vec3f(0.3 * side, 1.0, 7.0);
    b = a;
    sh.aim = vec3f(0.0, 0.0, 1.6);
    sh.lens = 0.5;
  }
  sh.pol = mix(a, b, 0.5 + (s - 0.5) * params.drift);
  return sh;
}

// Go to the director's shot: the view is the shot's (the `[[shot]]`s are the views in order), filmed
// from the side and at the distance the shot's seed gives, and the camera glides there from where
// it is over `len` seconds. A locked view ignores the director.
fn choose_view(index: u32, seed: f32, len: f32) {
  var v = f32(index);
  if (params.view_lock > 0.5) {
    v = params.view_lock - 1.0;
  }
  state.view = v;
  // The seed also sets the dutch angle, so the side comes from another digit of it.
  state.side = select(-1.0, 1.0, fract(seed * 7.31 + 0.37) < 0.5);
  // The circling view starts where the camera is.
  state.ring = state.pol.x;
  state.pol_from = state.pol;
  state.aim_from = state.aim_base;
  state.lens_from = state.lens;
  state.roll_from = state.roll;
  state.glide = 0.0;
  state.glide_len = len;
  state.travel = 0.0;
}

fn update() {
  let dt = rv_dt();
  let t = frame.time;
  if (frame.first == 1u) {
    state.level = 0.5;
    choose_view(shot.index, shot.seed, 1.0);
    state.cuts = shot.cuts;
    state.glide = 1.0;
    state.rec_bar = vec4f(-1.0);
    state.move_bar = -1000.0;
    state.move_gap = 1.0;
    state.gate_last = floor(input.clock / input.meter) - 1.0;
    state.hit_last = -1000.0;
  }

  let bar_len = input.meter * beat_len();
  let now_bar = input.clock / input.meter;
  if (frame.first == 1u) {
    state.bar_idx = floor(now_bar);
  }

  // The section's level (from the drive: the section's energy, or the loudness without a chart; the
  // runtime has smoothed it already) decides how often a big move comes, whether a backflip is
  // allowed and the moods; how hard they dance is the energy of the moment (spread over its
  // range, as Torii Sea's spirits) times how much is heard, so a quiet passage is a gentle dance, a
  // fade calms it and silence stills it.
  state.level = input.level * params.dance;
  state.amp = strength() * input.audible;

  // A bar is decided once, as it begins on the lead (a bar met in its middle, after a jump in
  // time, is left with no move rather than starting one half way).
  if (floor(now_bar) != state.gate_last) {
    let n = floor(now_bar);
    if (fract(now_bar) < 0.25) {
      decide_bar(n);
    } else {
      let slot = u32(max(n, 0.0)) % 4u;
      state.rec_bar[slot] = n;
      state.rec_move[slot] = 0.0;
      state.rec_dizzy[slot] = 0.0;
    }
    state.gate_last = n;
  }

  // The hits the groove is on where the beat is not followed: kept with their times (a hit within
  // half a beat of the last does not count), so that the chorus can dance them later in the wave.
  if (input.hit > 0.5 && t - state.hit_last >= 0.5 * beat_len()) {
    let slot = state.hit_n % 8u;
    state.hit_t[slot / 4u][slot % 4u] = t;
    state.hit_n += 1u;
    state.hit_last = t;
  }

  // Cues: a section start (0.5 or more) changes the mood, a lift of the energy (1) widens the
  // eyes; a weaker cue (a long stretch with no section) may change the mood.
  let new_section = input.new_section > 0.5;
  let cued = input.cue_weak > 0.5;
  if (new_section) {
    state.section = input.section;
  }
  // An impact (a hit far above the usual in the middle of a section) lifts the brows; a lift of
  // the energy lifts them too (the runtime fades both).
  state.brow = max(input.impact_env, input.lift_env);
  // Only a section that lifts the energy widens the eyes; an impact just lifts the brows.
  state.surprise = input.lift_env;

  // The mood: a section's (the same for sections that sound alike); on a weaker cue, a new one
  // that is kept while it still fits how hard they dance, and new figures to dance.
  if (frame.first == 1u) {
    state.mood = -1.0;
    choose_mood(state.level, 1.0, false);
    state.mood_prev = state.mood;
  } else if (new_section) {
    choose_mood(state.level, fract((input.label + 1.0) * 0.5698403 + 0.41), false);
  } else if (cued) {
    // New figures to dance: the section's number moves on by 101.
    state.section += 101.0;
    choose_mood(state.level, hash(vec2f(t, 2.1)), hash(vec2f(t, 5.3)) < 0.65);
  }

  // The camera: the director cuts (`shot.cuts` changes), and the camera glides to its shot over the
  // glide time, but never for longer than half the shot lasts. It cuts at the start of a section
  // itself, so no look-ahead is needed.
  if (shot.cuts != state.cuts) {
    state.cuts = shot.cuts;
    var glide_len = params.glide * bar_len;
    if (shot.length >= 0.0) {
      glide_len = min(glide_len, 0.5 * shot.length);
    }
    choose_view(shot.index, shot.seed, glide_len);
    // The burst's third cut films from the other side than its second.
    if (shot.fill == 2u) {
      state.burst_side = state.side;
    } else if (shot.fill == 3u) {
      state.side = -state.burst_side;
    }
    // A hard cut jumps there instead of gliding (`shot.hard`): inside a burst of cuts, where the
    // shots last a beat each; the cut that ends the burst, not a two-bar glide from the burst's
    // view from above; and the start of a high section, which the runtime dresses with a strong
    // transition (a hard cut is fine when a climax begins).
    if (shot.hard == 1u) {
      state.glide = 1.0;
    }
  }
  // The circling view goes round once in so many bars, the way of its side; it stops in silence.
  if (state.view > 4.5 && state.view < 5.5) {
    state.ring += state.side * dt * 6.28318 * input.audible / max(params.ring_bars * bar_len, 1.0);
  }
  // Each view moves slowly through the view's time, slowing to a stop; a glide goes round the
  // lead (bearing, distance and height each in turn), never through the dancers.
  state.glide = min(state.glide + dt / max(state.glide_len, 0.1), 1.0);
  // (A view's slow move goes by what is heard too, and stands in silence; a glide under way
  // finishes.)
  state.travel = min(state.travel + dt * input.audible / max(params.view_bars * bar_len, 1.0), 1.0);
  let s = 1.0 - (1.0 - state.travel) * (1.0 - state.travel);
  let to = view_shot(state.view, state.side, s);
  let k = smoothstep(0.0, 1.0, state.glide);
  let round_by = atan2(sin(to.pol.x - state.pol_from.x), cos(to.pol.x - state.pol_from.x));
  state.pol = vec3f(state.pol_from.x + round_by * k, mix(state.pol_from.yz, to.pol.yz, k));
  state.cam = vec3f(sin(state.pol.x) * state.pol.y, state.pol.z, -cos(state.pol.x) * state.pol.y);
  state.aim_base = mix(state.aim_from, to.aim, k);
  state.lens = mix(state.lens_from, to.lens, k);
  // The dutch angle of the shot, turned to along with the glide (it is fixed within a shot).
  // A locked view stays level.
  state.roll = mix(state.roll_from, select(shot.roll, 0.0, params.view_lock > 0.5), k);
  // The camera looks up after the lead when it hops or flips, a little late.
  state.lift += (pose_at(0.0, 1.0).hop - state.lift) * (1.0 - exp(-dt / 0.15));
  state.aim = state.aim_base + vec3f(0.0, 0.6 * state.lift, 0.0);

  // The dancers turn to face the camera when it has gone well round them, on a bar line, once the last turn has spread through the chorus. On the circling view
  // they turn a little past the camera, the way it goes, so as not to turn again too soon.
  let off = atan2(sin(state.pol.x - state.face_to), cos(state.pol.x - state.face_to));
  let settled = (now_bar - state.face_bar) * input.meter > params.turn_beats + 1.5;
  let due = floor(now_bar) != state.bar_idx;
  state.bar_idx = floor(now_bar);
  if (abs(off) > TURN && settled && due) {
    state.face_from = state.face_to;
    state.face_to += off + select(0.0, 0.6 * state.side, state.view > 4.5 && state.view < 5.5);
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
