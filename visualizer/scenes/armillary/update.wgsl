// Once per frame: the music moves the sphere as hands move a real one. Each ring is a pendulum, a
// body hung so it rests in its place: pushed, it swings out and comes back, slowed by friction;
// pushed hard, it goes over the top and on round a whole turn to its place again, a little past
// it and back. A push on one nudges its neighbours a moment later. A bar nudges one ring; a
// phrase of four bars flings one round a whole turn, each ring in its turn, each time the other
// way; a section flings three of them, one after another from the inside out, each the other way
// from the one before. Nothing steps with the beat. On the beats, the dots of a ring or (taking
// turns) a batch of shapes light and fade. The ecliptic precesses steadily. The planet at the
// heart turns and swells with the music. The camera cuts where the runtime's director says, to the
// shot it picks. Then the shapes are placed and projected, and the ones on the screen listed.

// ---- Colour ----
// Every colour in the picture comes from one: the theme colour, the player's own (which follows
// the system, the listener's choice or the cover) unless the setting says to use its own. Only its
// hue is kept, and how colourful it is (held down, so it is never loud at night, and taken down
// further by the grey look); every colour is made from them in OKLab, where a step in lightness
// or hue looks as big as it is: lines near white, the light warmer and fuller, the space round it
// all but black, a little turn of the hue here and there and nothing more.

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

// The same, as a tint: its brightest channel at 1 (to be scaled by how much light there is).
fn tint(lightness: f32, chroma: f32, hue: f32) -> vec4f {
  let c = oklch(lightness, chroma, hue);
  return vec4f(c / max(max(c.r, c.g), max(c.b, 1e-4)), 0.0);
}

// The theme's hue and (held down) chroma: of the player's accent colour once it has told it,
// else of the setting. The look then takes the chroma down (see `grade`).
fn theme() -> vec2f {
  let follow = params.follow == 1u && frame.accent.a > 0.5;
  let lab = oklab(select(params.theme.rgb, frame.accent.rgb, follow));
  return vec2f(atan2(lab.z, lab.y), min(length(lab.yz), 0.12));
}

// The looks (the runtime picks them; see the `[[look]]`s): the theme colour look has the colours
// as made, the grey look keeps the theme's hue with only a faint tint of its chroma, and a little
// dimmer (as Emoji Tunnel's and CRT Wall's greys are). Every colour in the picture is made from
// the theme's hue and chroma, so the looks are mapped here, where they are made, not graded over
// the finished picture. `state.colour` is how far the picture is in the theme colour look (0 the
// grey look, 1 the colour one), blending the two as the runtime fades from one to the other.
const GREY_TINT: f32 = 0.15;
const GREY_DIM: f32 = 0.85;

// The chroma the colours are made with, and the light they are scaled by.
fn grade_chroma(c: f32) -> f32 {
  return c * mix(GREY_TINT, 1.0, state.colour);
}

fn grade_light() -> f32 {
  return mix(GREY_DIM, 1.0, state.colour);
}

// The palette, by number: 0 the planet's characters and the lines; 1 the rings' light at rest (the
// dots, and the line of the spectrum); 2 what lighting adds to them, the same colour not dimmed by
// the grey look (it is light, not colour: the flare takes only highlights, so it must stay bright;
// the look's chroma is kept); 5 deep space; 6, 7 the stars' two tints. (8 to 13 went with the glass
// and bronze rings, the gem, the star and the lamp, 3 and 4 with the nebula; the numbers stay as
// they were.)
fn set_palette(th: vec2f) {
  let h = th.x;
  let c = grade_chroma(th.y);
  let dim = grade_light();
  state.palette[0] = tint(0.92, c * 0.45, h) * dim;
  state.palette[1] = tint(0.8, c, h) * dim;
  state.palette[2] = tint(0.8, c, h);
  state.palette[5] = vec4f(oklch(0.13, c * 0.25, h) * dim, 0.0);
  state.palette[6] = tint(0.93, c * 0.15, h - 0.5) * dim;
  state.palette[7] = tint(0.93, c * 0.25, h + 0.5) * dim;
  // Each ring the marks' colour, its hue turned a few degrees its own way.
  for (var i = 0u; i < RINGS; i++) {
    state.ring_tint[i] = tint(0.92, c * 0.45, h + (f32(i) - 2.0) * 0.07) * dim;
  }
}

fn random(n: u32) -> f32 {
  return rv_unit(frame.seed * 2654435761u + n);
}

// One step of a spring pulling x towards its target, with speed v: as stiff as `snap` and damped
// by `damping`, so it arrives fast and swings a little past.
fn spring(x: ptr<function, f32>, v: ptr<function, f32>, to: f32, dt: f32, snap: f32, damping: f32) {
  let k = snap * snap;
  let c = snap * 2.0 * damping;
  // In a few small steps, so a long frame cannot set it shaking.
  let n = 4;
  let h = dt / f32(n);
  for (var i = 0; i < n; i++) {
    *v += (k * (to - *x) - c * *v) * h;
    *x += *v * h;
  }
}

// The rings: outer radius and width, by number: 0 the horizon, 1 the meridian, 2 the ecliptic,
// 3 the sun's ring, 4 an outer ring round them all (the moon's ring, once the innermost, sat too
// close to the core). The marks lie along the middle of the band (a radius less half the width).
fn ring_size(i: u32) -> vec2f {
  var sizes = array<vec2f, 5>(vec2f(3.5, 0.22), vec2f(2.85, 0.15), vec2f(2.2, 0.24), vec2f(1.6, 0.08), vec2f(4.2, 0.12));
  return sizes[i];
}

// Where ring i rests: its axis, and a line in its plane.
fn home_a(i: u32) -> vec3f {
  let p = pole();
  var a = array<vec3f, 5>(vec3f(0.0, 1.0, 0.0), vec3f(1.0, 0.0, 0.0), turn(p, vec3f(1.0, 0.0, 0.0), radians(23.44)), normalize(vec3f(0.3, 0.9, 0.3)), normalize(vec3f(-0.5, 0.4, 0.75)));
  return a[i];
}

fn home_b(i: u32) -> vec3f {
  let p = pole();
  var b = array<vec3f, 5>(vec3f(0.0, 0.0, 1.0), p, cross(p, vec3f(1.0, 0.0, 0.0)), vec3f(0.0, 0.0, 1.0), vec3f(1.0, 0.0, 0.0));
  return b[i];
}

// How ring i swings and turns, all in proportion to its time (the bigger, the slower): how hard
// it is pulled back to its place, how long it coasts, how long a push on it lasts. Tuned so a
// whole turn goes a little past its place and back, and lands there for any push from about
// 4 to 10 (TURN is in the middle).
const HOLD: f32 = 4.0;
const COAST: f32 = 0.12;
const DRIVE: f32 = 0.25;
const TURN: f32 = 6.4;

fn ring_time(i: u32) -> f32 {
  return params.whirl_time * mix(0.8, 1.25, (ring_size(i).x - 1.6) / 2.6);
}

// A push of about `angle` on ring i (the angle it would turn if nothing held it back), now.
fn push(i: u32, angle: f32) {
  let time = ring_time(i);
  state.ring_turn[i].z += angle / (COAST * time * DRIVE * time);
}

// The same, landing `delay` seconds from now; or (`whole`) a whole turn then, `angle` then its
// way round and `across` the line on the screen it tumbles about (see whirl).
fn push_later(i: u32, angle: f32, delay: f32, whole: bool, across: f32) {
  state.ring_wait[i] = vec4f(angle, max(delay, 1e-4), f32(whole), across);
}

// Is ring i in its place (near enough), not on its way round?
fn settled(i: u32) -> bool {
  return abs(sin(state.ring_turn[i].x * 0.5)) < 0.25;
}

// Ring i is flung round a whole turn, `way` (1 or -1) round: where it is now becomes where it
// rests, and it is flung about a new axis, as near as its plane allows to the line on the screen
// at angle `across`, so it tumbles over plainly and rings flung together tumble alike, each its
// own way round. (Not while it is already on its way round.)
fn whirl(i: u32, way: f32, across: f32) {
  if (!settled(i)) {
    return;
  }
  let m = state.ring_turn[i];
  let about = state.ring_about[i].xyz;
  let a = turn(state.ring_rest_a[i].xyz, about, m.x);
  let b = turn(state.ring_rest_b[i].xyz, about, m.x);
  state.ring_rest_a[i] = vec4f(a, 0.0);
  state.ring_rest_b[i] = vec4f(b, 0.0);
  let line = state.cam_right * cos(across) + state.cam_up * sin(across);
  var onplane = line - a * dot(line, a);
  if (length(onplane) < 0.3) {
    onplane = cross(a, state.cam_fwd);
  }
  let axis = normalize(normalize(onplane) + a * 0.3) * way;
  state.ring_about[i] = vec4f(axis, 0.0);
  state.ring_turn[i] = vec4f(0.0);
  push(i, TURN);
}

// Ring k flung round `way`, its neighbours nudged after it the other way, less the further they
// are.
fn whirl_one(k: u32, way: f32, across: f32) {
  whirl(k, way, across);
  for (var i = 0u; i < RINGS; i++) {
    let apart = abs(f32(i) - f32(k));
    if (apart > 0.5 && apart < 2.5) {
      push_later(i, -way * select(0.15, 0.4, apart < 1.5), 0.08 * apart, false, 0.0);
    }
  }
}

// Ring i moving for dt: a push waiting for it landing; its push dying away, its speed driven by
// the push, slowed by friction and pulled back towards its place (or the next one round).
fn move_ring(i: u32, dt: f32) {
  var w = state.ring_wait[i];
  if (w.y > 0.0) {
    w.y -= dt;
    state.ring_wait[i] = select(w, vec4f(0.0), w.y <= 0.0);
    if (w.y <= 0.0) {
      if (w.z > 0.5) {
        whirl(i, w.x, w.w);
      } else {
        push(i, w.x);
      }
    }
  }
  var m = state.ring_turn[i];
  let time = ring_time(i);
  let hold = (HOLD / time) * (HOLD / time);
  let coast = COAST * time;
  let drive = DRIVE * time;
  let n = 4;
  let h = dt / f32(n);
  for (var k = 0; k < n; k++) {
    m.y += (m.z - m.y / coast - hold * sin(m.x)) * h;
    m.x += m.y * h;
    m.z *= exp(-h / drive);
  }
  state.ring_turn[i] = m;
  // Its place eases back to where it first rested, so the sphere keeps its shape over a long
  // set.
  let ease = 1.0 - exp(-dt / 8.0);
  state.ring_rest_a[i] = vec4f(normalize(mix(state.ring_rest_a[i].xyz, home_a(i), ease)), 0.0);
  state.ring_rest_b[i] = vec4f(normalize(mix(state.ring_rest_b[i].xyz, home_b(i), ease)), 0.0);
}

// A ring's frame from its axis and a direction in its plane (where its first mark lies).
fn ring_frame(i: u32, axis: vec3f, along: vec3f) {
  let size = ring_size(i);
  let z = normalize(axis);
  let x = normalize(along - z * dot(along, z));
  let y = cross(z, x);
  state.ring_x[i] = vec4f(x, size.x);
  state.ring_y[i] = vec4f(y, size.y);
  state.ring_z[i] = vec4f(z, 0.0);
}

// Every ring in its place, still; a nudge before its first turn tilts it about a line in its
// plane.
fn rest_rings() {
  for (var i = 0u; i < RINGS; i++) {
    state.ring_rest_a[i] = vec4f(home_a(i), 0.0);
    state.ring_rest_b[i] = vec4f(home_b(i), 0.0);
    state.ring_about[i] = vec4f(normalize(home_b(i)), 0.0);
    state.ring_turn[i] = vec4f(0.0);
    state.ring_wait[i] = vec4f(0.0);
  }
}

// The rings now, from how each is moving; the ecliptic precesses about the pole.
fn set_rings() {
  for (var i = 0u; i < RINGS; i++) {
    let m = state.ring_turn[i];
    let about = state.ring_about[i].xyz;
    var a = turn(state.ring_rest_a[i].xyz, about, m.x);
    var b = turn(state.ring_rest_b[i].xyz, about, m.x);
    if (i == 2u) {
      a = turn(a, pole(), input.spin);
      b = turn(b, pole(), input.spin);
    }
    ring_frame(i, a, b);
  }
}

// ---- Accents ----
// The rings rest as faint dots and an accent lights one, all its dots together (see view.wgsl),
// fading back over a moment; a strong one lights two. Or it lights a batch of the shapes.

// Ring i lit as strongly as `strength`: the dots round `light_reach` of the ring (from somewhere
// new) brighten together, then dim together; one still lit brightens from where it is.
fn light_ring(i: u32, strength: f32, n: u32) {
  let on = state.ring_on[i];
  state.ring_on[i] = vec4f(random(60u + n), on.y, max(on.z, strength), 0.5 * params.light_reach);
}

// The rings in the order they light, an order that leaps about: the ecliptic, the horizon, the
// sun's, the meridian, the outer ring. The ring a lighting `n` lights, and (a strong one) the one
// it lights with it. Neither is one that the lighting before lit: the first is the next in the
// order, the second two on, and a lighting and the next are (n, n + 2) and (n + 1, n + 3), which
// share none of five. The kick's lighting takes the next number too, so it keeps to the same rule
// (two lightings in a row must never be the same ring).
fn ring_lit_first(n: u32) -> u32 {
  var order = array<u32, 5>(2u, 0u, 3u, 1u, 4u);
  return order[n % 5u];
}

fn ring_lit_second(n: u32) -> u32 {
  var order = array<u32, 5>(2u, 0u, 3u, 1u, 4u);
  return order[(n + 2u) % 5u];
}

// An accent as strong as `strength` lights whichever of the two is lighting now: the next ring
// in an order that leaps about, and on a strong one another with it; or a batch of the shapes.
fn light(strength: f32) {
  state.lits += 1.0;
  let n = u32(state.lits);
  if (state.layer < 0.5) {
    light_ring(ring_lit_first(n), strength, 0u);
    if (strength > 0.85) {
      light_ring(ring_lit_second(n), strength * 0.7, 1u);
    }
    return;
  }
  light_shapes(strength, n);
}

// A batch of the shapes, the next batch in turn (`n` counts the lightings), each made anew as it
// lights (somewhere else, another shape): unlit they are not there, so no one sees them change.
// The batch before gives way to it, dimming at once to a third and fading on from there.
fn light_shapes(strength: f32, n: u32) {
  let batch = n % BATCHES;
  state.last_batch = f32(batch);
  let tip = (vec3f(random(50u), random(51u), random(52u)) - 0.5) * 1.2;
  state.batch_axis[batch] = vec4f(normalize(-state.cam_fwd + tip), 0.0);
  // How many: never the same, more on a stronger beat. How big: as they are close up, up to
  // twice that seen from far off, so they look much the same size in every shot.
  let most = f32(IN_BATCH) * params.shapes;
  let count = clamp(round(most * mix(0.35, 1.0, random(53u)) * mix(0.7, 1.0, strength)), 2.0, f32(IN_BATCH));
  let far = clamp(length(state.cam_pos - CENTRE) / 8.0, 1.0, 2.0);
  state.batch_look[batch] = vec4f(count, far, 0.0, 0.0);
  for (var i = 0u; i < SHAPES; i++) {
    if (i % BATCHES == batch) {
      state.shape_look[i].z += 1.0;
      state.shape_look[i].y = strength;
    } else {
      state.shape_look[i].y *= 0.35;
    }
  }
}

// A kick between the beats lights as strongly as `strength`, of its own. While the rings are
// lighting: the next ring in the order, as a beat's light does (never the one lit just before, and
// it takes the next number, so the next beat does not light it either). While the shapes are: the
// last batch again, if that is above what it has now (a shape that has faded out is not there; it
// is placed again only when its batch is lit).
fn kick_light(strength: f32) {
  if (state.layer < 0.5) {
    state.lits += 1.0;
    light_ring(ring_lit_first(u32(state.lits)), strength, 0u);
    return;
  }
  if (state.last_batch < -0.5) {
    return;
  }
  let batch = u32(state.last_batch);
  for (var i = 0u; i < SHAPES; i++) {
    if (i % BATCHES == batch && state.shape_look[i].y >= 0.02 && strength > state.shape_look[i].y) {
      state.shape_look[i].y = strength;
    }
  }
}

// Shape i: where it hangs and how it lies, made anew each time its batch is lit (from how many
// times that is). A polygon of three to six sides, hanging just outside the outermost ring; the
// shapes of a batch (how many, and how big for the camera's distance, set as it lit) spaced
// unevenly round a band about the batch's axis (facing the camera as the batch lit, tipped a
// little), so together they ring the sphere round as it is seen, none in front of it; each
// facing the middle more or less, turning slowly where it is.
fn place(i: u32, th: vec2f) {
  let batch = i % BATCHES;
  let place_in = i / BATCHES;
  let made = state.batch_look[batch];
  let many = max(u32(made.x), 1u);
  let present = f32(place_in < many);
  let id = vec2f(f32(i), 3.7 + state.shape_look[i].z * 17.31);
  let band_id = vec2f(f32(batch), 5.3 + state.shape_look[i].z * 17.31);
  let t = state.phase;
  // The batch's band, its shapes round it, unevenly spaced, some well above or below it.
  let axis = state.batch_axis[batch].xyz;
  let e1 = normalize(cross(axis, select(vec3f(0.0, 1.0, 0.0), vec3f(1.0, 0.0, 0.0), abs(axis.y) > 0.9)));
  let e2 = cross(axis, e1);
  let round_at = hash(band_id + 0.3) * 6.2831853 + (f32(place_in) + 0.85 * (hash(id + 0.4) - 0.5)) * 6.2831853 / f32(many);
  let lift = (hash(id + 0.2) - 0.5) * 1.0;
  let out = normalize((e1 * cos(round_at) + e2 * sin(round_at)) * cos(lift) + axis * sin(lift));
  let r = mix(4.6, 6.1, hash(id + 0.1));
  let size = mix(0.25, 0.95, pow(hash(id + 0.5), 1.5)) * made.y * params.shape_size;
  state.shape_pos[i] = vec4f(CENTRE + out * r, size * present);
  // Facing the middle, tipped (some well over), turning slowly in its own plane.
  let n = normalize(out + (vec3f(hash(id + 2.1), hash(id + 2.2), hash(id + 2.3)) - 0.5) * 1.1);
  let u0 = normalize(cross(n, select(vec3f(0.0, 1.0, 0.0), vec3f(1.0, 0.0, 0.0), abs(n.y) > 0.9)));
  let angle = t * mix(0.12, 0.3, hash(id + 2.7)) * select(-1.0, 1.0, hash(id + 0.3) < 0.5) + hash(id + 2.8) * 6.2831853;
  let sides = 3.0 + floor(hash(id + 2.9) * 4.0);
  state.shape_n[i] = vec4f(n, sides);
  state.shape_u[i] = vec4f(turn(u0, n, angle), 0.0);
  // The lines' colour, a little lighter or darker, its hue turned up to ten degrees either way.
  state.shape_tint[i] = tint(0.92 + 0.05 * (hash(id + 3.1) - 0.5), grade_chroma(th.y) * 0.5, th.x + (hash(id + 3.2) - 0.5) * 0.35) * grade_light();
}

// Where shape i is on the screen and how far it reaches (kept for the pixels), and, if it is to
// be drawn, the entry for the list of drawn shapes: its centre and reach on the screen and its
// number; else a reach below zero.
fn project(i: u32) -> vec4f {
  let none = vec4f(0.0, 0.0, -1.0, 0.0);
  let p = state.shape_pos[i];
  if (p.w <= 0.0) {
    state.shape_scr[i] = vec4f(0.0, 0.0, -1.0, -1.0);
    return none;
  }
  let v = p.xyz - state.cam_pos;
  let zc = dot(v, state.cam_fwd);
  // (Not one right by the lens: it would be a blur across the picture.)
  // (The burst's cut at a polygon stands nearer than that.)
  if (zc < select(3.0, 0.6, state.view > 4.5 && state.view < 5.5)) {
    state.shape_scr[i] = vec4f(0.0, 0.0, -1.0, -1.0);
    return none;
  }
  let aspect = frame.aspect;
  let c = state.focal * vec2f(dot(v, state.cam_right), dot(v, state.cam_up)) / zc;
  // As far as its corners, and enough pixels for its line and the tail of its glow to die away
  // (the square a pixel is tested against has no soft edge, as the tiles' edges did not need).
  let rad = state.focal * p.w * 1.05 / zc + 10.0 / frame.resolution.y;
  state.shape_scr[i] = vec4f(c, rad, zc);
  // (Off the screen it is not drawn.)
  if (abs(c.x) - rad > aspect * 0.5 || abs(c.y) - rad > 0.5) {
    return none;
  }
  return vec4f(c, rad, f32(i));
}

// ---- The burst ----
// The burst of four hard cuts before a section that lifts (`shot.fill` 1 to 4, the shots 4 to 7).
// Each is another way of looking, and its subject is chosen here, at the cut, and kept for the cut:
// 1 along a ring, close beside it; 2 at a polygon, lit for it, seen 30 degrees off its face; 3 down
// the pole, the rings as ellipses round the planet; 4 the planet, filling most of the picture.
// What a cut frames is put at the picture's middle at a distance worked out from its size, so
// that the dutch angle (a turn about the camera's axis) cannot lose it; its size is measured on
// the shorter side of the picture (`frame_unit`), so it also fits a portrait one.

// A random number that differs from cut to cut (`random` is the same all through a run).
fn random_cut(n: u32) -> f32 {
  return rv_unit(frame.seed * 2654435761u + n * 40503u + shot.cuts * 7919u + 977u);
}

// The shorter side of the picture, in units of its height.
fn frame_unit() -> f32 {
  return min(1.0, frame.aspect);
}

// How far a camera of focal length `f` stands from a sphere of radius `r` for it to show with the
// radius `rho` on the screen (units of the picture's height).
fn frame_dist(r: f32, rho: f32, f: f32) -> f32 {
  let reach = f * r / rho;
  return sqrt(r * r + reach * reach);
}

fn burst_ring_radius(i: u32) -> f32 {
  return ring_size(i).x - ring_size(i).y * 0.5;
}

fn choose_burst(fill: u32, th: vec2f) {
  let u = frame_unit();
  let f = params.lens;
  let up = vec3f(0.0, 1.0, 0.0);
  state.burst_off = vec3f(0.0);
  state.burst_hold = 0.0;
  if (fill == 1u) {
    // Along a ring: the ring the order lights next, lit now as a beat's accent would, in full (and
    // the beat's own lighting is left out this frame: see update), so it is the bright subject.
    // The order never lights the ring the lighting just before lit, so the ring that was lit
    // last is never this one. It is held lit for the cut, from a place round it that the cut's
    // own number picks.
    state.lits += 1.0;
    let ring = ring_lit_first(u32(state.lits));
    light_ring(ring, 1.0, 0u);
    state.burst_a = f32(ring);
    state.burst_phi = random_cut(1u) * 6.2831853;
    state.burst_hold = 1.0;
    state.burst_seen = vec3f(0.0);
  } else if (fill == 2u) {
    // At a polygon: a batch lights now, as a beat's accent would (and the beat's own lighting is
    // left out this frame: see update), the band of it facing the side the camera will be on;
    // one of it is picked by the cut's number; the camera stands on the outer side of it, 30
    // degrees off its face, at the distance that gives its outline a radius of 0.36 of the
    // picture's short side. (The camera's place is set early here, for the batch to face it.)
    let side = vec3f(cos(random_cut(2u) * 6.2831853), 0.25, sin(random_cut(2u) * 6.2831853));
    state.cam_fwd = -normalize(side);
    state.cam_pos = CENTRE + normalize(side) * 9.0;
    state.lits += 1.0;
    light_shapes(mix(0.55, 1.0, input.energy), u32(state.lits));
    let batch = u32(state.last_batch);
    let many = max(u32(state.batch_look[batch].x), 1u);
    let pick = batch + BATCHES * min(u32(random_cut(3u) * f32(many)), many - 1u);
    for (var k = 0u; k < many; k++) {
      place(batch + BATCHES * k, th);
    }
    let p = state.shape_pos[pick];
    let out = normalize(p.xyz - CENTRE);
    let n = state.shape_n[pick].xyz;
    let face = n * select(-1.0, 1.0, dot(n, out) >= 0.0);
    var across = cross(face, up);
    if (length(across) < 0.2) {
      across = cross(face, vec3f(1.0, 0.0, 0.0));
    }
    state.burst_dir = normalize(face * 0.8660254 + normalize(across) * state.side * 0.5);
    state.burst_a = f32(pick);
    state.burst_dist = frame_dist(p.w, 0.36 * u, f);
    state.burst_hold = 1.0;
    state.burst_seen += normalize(p.xyz + state.burst_dir * state.burst_dist - CENTRE);
  } else if (fill == 3u) {
    // Down the pole: far enough that the outer ring (with its width) fills 0.88 of the short side.
    state.burst_dir = pole();
    state.burst_dist = frame_dist(ring_size(4u).x, 0.44 * u, f);
    state.burst_seen += pole();
  } else {
    // The planet: from the side the burst has looked at least, a little off its middle, filling
    // 0.7 of the short side.
    let r = planet_radius();
    let wander = normalize(vec3f(random_cut(5u) - 0.5, random_cut(6u) - 0.5, random_cut(7u) - 0.5) + vec3f(0.0, 0.0, 0.01));
    state.burst_dir = normalize(select(wander, wander * 0.35 - normalize(state.burst_seen), length(state.burst_seen) > 0.2));
    state.burst_dist = frame_dist(r, 0.35 * u, f);
    var across = cross(state.burst_dir, up);
    if (length(across) < 0.2) {
      across = vec3f(1.0, 0.0, 0.0);
    }
    state.burst_off = normalize(across) * 0.12 * r + up * 0.05 * r;
  }
}

fn update() {
  let dt = rv_dt();
  if (frame.first == 1u) {
    state.core_size = 1.0;
    rest_rings();
    state.spec_ring = 4.0;
    state.last_batch = -1.0;
    // No beat has been counted yet.
    state.beat_age = 1000.0;
  }
  // What the music drives is declared in the manifest (`[[input]]` and `[[bind]]`): how much is
  // heard (`input.audible`), the energy of the moment (`input.energy`), how far the bass is above
  // its average (`input.core_bass`), the tempo to move at (`input.pace_tempo`, always there: a calm
  // one when the music has no beat, held at 40 or more) and the moments (`input.hit`,
  // `input.bar_line`, `input.section`, one-frame pulses).
  let beat_time = 60.0 / input.pace_tempo;

  // How far the picture is in the theme colour look: the runtime's fade from look.prev to look.to
  // (0 the grey look, 1 the colour one; the looks are in the order of the `[[look]]`s). No setting
  // pins a look: the lab holds one.
  state.colour = select(0.0, 1.0 - look.mix, look.prev == 1u) + select(0.0, look.mix, look.to == 1u);

  // How much is heard, and the scene's own time by it: the phase goes on by `dt` times
  // `input.audible` (smoothed in its bind, so it slows with a song's own fade), so that everything
  // that turns of itself (the shapes' turn in their planes) stands still in silence; the paces the
  // runtime sums (`input.spin`, `input.cam_drift`, `input.planet_turn`) are multiplied by it in
  // their binds. Only while something is heard (above
  // 0.3) does a discrete move happen (a beat counted, a bar's nudge, a phrase's whole turn, a
  // section's three rings, the lighting, the planet's swell), so a stray onset in near silence
  // moves nothing; a push already given (a pendulum settling back, a push landing after its delay)
  // is let finish. The stars' twinkle stays on the clock (it is light, not motion).
  state.phase += dt * input.audible;
  let heard = input.audible > 0.3;

  // The beats, counted here. `input.hit` is a heard hit: at level A each beat followed, at B and C
  // every onset (the kick, snare and hat, which can be dense). So a beat counts only if at least
  // half a beat has passed since the last counted one and something is heard. Everything that
  // goes by the beat (the planet's swell, the accents, the beat of the bar) uses the counted
  // beats only.
  state.beat_age = min(state.beat_age + dt, 1000.0);
  let beat = input.hit > 0.5 && heard && state.beat_age >= 0.5 * beat_time;
  if (beat) {
    state.count += 1.0;
    state.beat_age = 0.0;
    state.core_vel += params.core_kick;
  }
  // Bar lines: where the beat is followed (`input.followed`, level A), the music's own
  // (`input.bar_line`); everywhere else (levels B and C, which have none) every fourth counted
  // beat. Either way the beat of the bar starts again on the bar line used.
  let followed = input.followed > 0.5;
  let barred = select(beat && state.beat_in_bar + 1.0 >= 4.0, input.bar_line > 0.5 && heard, followed);

  // Rings move by the bar, the phrase and the section, never step by step on the beat: a bar
  // nudges one ring, each in its turn; a phrase of four bars flings one round a whole turn, each
  // in its turn, its neighbours nudged after; a section flings three, from the inside out. A
  // cue strong enough (`cue_min`) is the moment to fling rings.
  let section = input.section > 0.5 && frame.first == 0u && heard;
  if (barred) {
    state.bars += 1.0;
    let bar = u32(state.bars);
    if (bar % 4u == 0u) {
      state.phrases += 1.0;
      // Another ring (never the ecliptic, which shows the spectrum in its ticks, nor the one
      // before) becomes the line of the spectrum.
      var others = array<u32, 4>(0u, 1u, 3u, 4u);
      var pick = others[u32(random(60u) * 4.0) % 4u];
      if (f32(pick) == state.spec_ring) {
        pick = others[(u32(random(60u) * 4.0) + 1u + u32(random(61u) * 3.0)) % 4u];
      }
      state.spec_ring = f32(pick);
      if (!section) {
        // The rings in an order that leaps about: the moon's, the ecliptic, the horizon, the
        // sun's, the meridian.
        var order = array<u32, 5>(4u, 2u, 0u, 3u, 1u);
        let way = select(-1.0, 1.0, u32(state.phrases) % 2u == 0u);
        whirl_one(order[u32(state.phrases) % 5u], way, random(4u) * 6.2831853);
      }
    } else {
      let k = (bar * 2u + 1u) % RINGS;
      // The push is as big as the energy is (0.6 to 1.4 times).
      push(k, radians(params.sway) * 1.5 * select(-1.0, 1.0, bar % 2u == 0u) * mix(0.6, 1.4, input.energy));
    }
  }
  if (section) {
    state.sections += 1.0;
    // Three of the rings (never all), a different three each section, one after another from
    // the inside out, each tumbling about a different line on the screen, a little over a third
    // of a turn from the last, each the other way from the last.
    let across = random(5u) * 6.2831853;
    let first = u32(state.sections) % RINGS;
    for (var k = 0u; k < 3u; k++) {
      let i = (first + 2u * k) % RINGS;
      // (How far in it is: how many of the rings are smaller.)
      var inner = 0.0;
      for (var m = 0u; m < RINGS; m++) {
        inner += f32(ring_size(m).x < ring_size(i).x);
      }
      push_later(i, select(-1.0, 1.0, k % 2u == 0u), 0.12 * inner, true, across + f32(k) * 1.885);
    }
  }
  if (barred) {
    state.beat_in_bar = 0.0;
  } else if (beat) {
    state.beat_in_bar += 1.0;
  }
  // The rings and the shapes take turns to light, every `layer_bars` bars.
  if (barred && u32(state.bars) % u32(max(params.layer_bars, 1)) == 0u) {
    state.layer = 1.0 - state.layer;
  }
  state.layer_mix += (state.layer - state.layer_mix) * (1.0 - exp(-dt / 0.4));
  for (var i = 0u; i < RINGS; i++) {
    move_ring(i, dt);
  }

  // The director cuts to another shot: its index is the view and its seed picks the side. (The
  // cut count is compared with the one seen, so the first frame frames the first shot too.) A cut of
  // the burst (the views 4 to 7) chooses what it frames now (`choose_burst`), the first frame of it.
  var burst_lit = false;
  if (frame.first == 1u || shot.cuts != u32(state.cuts_seen)) {
    state.cuts_seen = f32(shot.cuts);
    state.view = f32(shot.index);
    state.side = select(-1.0, 1.0, shot.seed < 0.5);
    if (shot.fill > 0u) {
      choose_burst(shot.fill, theme());
      burst_lit = shot.fill <= 2u;
    }
  }
  let view = state.view;

  // Accents fall on the counted beats: the bar's first beat strongest, its third next, the others
  // least, all of it stronger with the energy (0.55 times in the quietest passage, whole in the
  // loudest, so in a quiet passage only the first and third beats pass `accent`). What is lit
  // fades by the beat: by half in `lit_beats` beats.
  if (beat) {
    let at = u32(state.beat_in_bar) % 4u;
    let weight = select(select(0.5, 0.75, at == 2u), 1.0, at == 0u);
    let accent = weight * mix(0.55, 1.0, input.energy);
    // (The burst's cut at a polygon has lit a batch of its own this frame, on the beat it is on.)
    if (accent > params.accent && !burst_lit) {
      light(accent);
    }
  }
  // The kicks between the beats, where the beat is followed (`input.kick` is 0 elsewhere, its bind
  // multiplies by `followed`: at levels B and C the counted beats already are the heard onsets,
  // the kick among them). A kick is an onset: the envelope jumps up by more than 0.3 from the
  // frame before. Taken only while something is heard, and between a quarter and three quarters
  // of a beat after the last counted beat (so not the kick on a beat, which that beat has lit, nor
  // one just ahead of the next, which the next will light): it is a lighting of its own, as
  // strong as `kick_light` times the kick's strength times the energy's factor (see
  // `kick_light()`: the next ring, never the one lit just before; or the last batch again).
  let jump = input.kick - state.kick_prev;
  state.kick_prev = input.kick;
  if (jump > 0.3 && heard && !beat && state.beat_age >= 0.25 * beat_time && state.beat_age <= 0.75 * beat_time) {
    kick_light(params.kick_light * input.kick * mix(0.55, 1.0, input.energy));
  }
  let fade = exp(-dt * 0.693 / (params.lit_beats * beat_time));
  // A lit ring's dots fall with a time constant of `light_beats` beats (a quarter; the dots take
  // 1/6 beat to rise and 1/4 to fall).
  let on_fade = exp(-dt / (params.light_beats * beat_time));
  for (var i = 0u; i < RINGS; i++) {
    // The dots brighten all together, quickly at first and slower as they near the accent's
    // strength (a time constant of a third of `light_spread` beats), and dim together as it fades.
    var on = state.ring_on[i];
    // (The ring the burst looks along is held lit for its cut.)
    if (!(state.burst_hold > 0.5 && view > 3.5 && view < 4.5 && i == u32(state.burst_a))) {
      on.z *= on_fade;
    }
    let rise = 1.0 - exp(-dt * 3.0 / max(params.light_spread * beat_time, 0.01));
    on.y = select(on.z, on.y + (on.z - on.y) * rise, on.z > on.y);
    state.ring_on[i] = on;
    // The ring of the spectrum eases out of its circle, the one before back into it.
    let to = f32(f32(i) == state.spec_ring);
    state.ring_spec[i].x += (to - state.ring_spec[i].x) * (1.0 - exp(-dt / 0.6));
  }

  // The planet: as big as the energy is (with the bass), springing out on each counted beat and
  // back.
  let big = 1.0 + 0.35 * input.energy + 0.12 * input.core_bass;
  var size = state.core_size;
  var size_vel = state.core_vel;
  spring(&size, &size_vel, big, dt, 12.0, 0.35);
  state.core_size = size;
  state.core_vel = size_vel;

  // Between the pushes everything keeps going, each at its own pace, all of it quicker as the
  // music is louder: the ecliptic's precession, the camera, and the character-art planet, which
  // turns with the tempo (a turn in about 24 beats). The runtime sums these (`input.spin`,
  // `input.cam_drift`, `input.planet_turn`: the pace of each, by the second, times its slider, in
  // radians); the shaders read them as they are. The camera's sum starts anew with each shot
  // (`reset = "cut"`).

  // The rings first: the camera of a burst's cut at a ring follows the ring as it swings.
  set_rings();

  // The shot, about the sphere in space: 0 a little below, pushing in; 1 level, from the side,
  // the rings seen nearly edge on, circling; 2 close, the rings crossing, circling; 3 from above,
  // circling. Each mirrored by its side; the camera moves slowly and evenly, never with the beat.
  // The burst's cuts, 4 to 7 (see `choose_burst`): along a ring, at a polygon, down the pole, at
  // the planet; each framed from what was chosen at the cut, and pushing in a few percent over its
  // length, by how much is heard.
  let c = CENTRE;
  let s = state.side;
  let held = input.cam_drift;
  let round = s * held;
  let up = vec3f(0.0, 1.0, 0.0);
  var upward = up;
  var cam: vec3f;
  var aim: vec3f;
  var focal: f32;
  let push = 0.04 * smoothstep(0.0, 1.0, clamp(shot.age / max(shot.length, 0.25), 0.0, 1.0)) * input.audible;
  if (view < 0.5) {
    aim = c + vec3f(0.0, 0.3, 0.0);
    cam = mix(c + vec3f(4.4 * s, -1.8, -5.6), aim, 0.3 * (1.0 - exp(-held / 30.0)));
    focal = 0.9;
  } else if (view < 1.5) {
    // Level with the middle, from a side (20 degrees off the line of the meridian's axis), nearer
    // than the first (the focal length over the distance is 0.167 against 0.123): the horizon ring
    // is a line across the picture and the others cross it as tilted lines and narrow ellipses.
    cam = c + turn(vec3f(5.64 * s, 0.0, -2.05), up, round * 0.015);
    aim = c;
    focal = 1.0;
  } else if (view < 2.5) {
    cam = c + turn(vec3f(2.6 * s, 1.0, -3.4), up, round * 0.02);
    aim = c + turn(vec3f(-0.3 * s, -0.2, 0.0), up, round * 0.02);
    focal = 1.0;
  } else if (view < 3.5) {
    cam = c + turn(vec3f(1.6 * s, 8.0, -4.2), up, round * 0.015);
    aim = c;
    focal = 1.0;
  } else if (view < 4.5) {
    // Along a ring, the ring the cut lit: the camera 1.2 m outside it (in its plane's outward
    // direction) and 0.5 m off its plane, looking along its tangent turned in towards the middle,
    // so the arc sweeps from big marks near the lens across to the far side with the planet in
    // the picture. The angle is chosen by the picture's shape, with the planet about 35 degrees
    // off the view axis in a wide picture (ring plane level, so the arc sweeps across) and 25 in a
    // tall one (the ring's inward side up, so it sweeps from the bottom up), both inside the
    // lens's field; a wide lens, which the lens setting does not change.
    let i = u32(state.burst_a);
    let x = state.ring_x[i].xyz;
    let y = state.ring_y[i].xyz;
    let z = state.ring_z[i].xyz;
    let out = x * cos(state.burst_phi) + y * sin(state.burst_phi);
    let along = (y * cos(state.burst_phi) - x * sin(state.burst_phi)) * s;
    let zup = z * select(-1.0, 1.0, dot(z, up) >= 0.0);
    let tall = frame.aspect < 1.0;
    let turn_in = select(0.9599, 1.1345, tall);
    let heading = normalize(along * cos(turn_in) - out * sin(turn_in));
    cam = c + out * (burst_ring_radius(i) + 1.2) + zup * 0.5 + heading * (2.0 * push);
    aim = cam + heading * 6.0 - zup * 0.5;
    upward = select(zup, -out, tall);
    focal = select(0.6, 0.55, tall);
  } else if (view < 5.5) {
    // At the polygon the cut lit.
    let p = state.shape_pos[u32(state.burst_a)].xyz;
    aim = p;
    cam = p + state.burst_dir * state.burst_dist * (1.0 - push);
    focal = params.lens;
  } else if (view < 6.5) {
    // Down the pole.
    aim = c;
    cam = c + state.burst_dir * state.burst_dist * (1.0 - push);
    upward = normalize(up - state.burst_dir * dot(up, state.burst_dir));
    focal = params.lens;
  } else {
    // The planet.
    aim = c + state.burst_off;
    cam = c + state.burst_dir * state.burst_dist * (1.0 - push);
    focal = params.lens;
  }
  // Further back with a longer lens, the same framing with less of the near side of each ring
  // swelling: concentric rings look concentric. (The burst's cuts are framed with it already.)
  if (view < 3.5) {
    cam = aim + (cam - aim) * params.lens;
    focal *= params.lens;
  }
  let fwd = normalize(aim - cam);
  let level_right = normalize(cross(upward, fwd));
  let level_up = cross(fwd, level_right);
  // The director's dutch angle, fixed within a shot: the camera turns about its own axis.
  let roll = shot.roll;
  state.cam_pos = cam;
  state.cam_fwd = fwd;
  state.cam_right = level_right * cos(roll) + level_up * sin(roll);
  state.cam_up = level_up * cos(roll) - level_right * sin(roll);
  state.focal = focal;
  // What is bright fringes a little red and blue, as on a screen: fixed (about what the beat's
  // widening averaged at 120 bpm, 0.0022 * (1 + 0.43)), not following any beat.
  post.chromatic = 0.003;

  // (The theme is worked out once, for the palette and for every shape's colour.)
  let th = theme();
  set_palette(th);
  // The shapes fade with the beat; one that is not lit is not there (it is placed again when it
  // lights), so it is passed by. The rest are placed and projected, and listed if they are to be
  // drawn, in the order of their numbers: one write each, however big they are on the screen.
  var hits = 0u;
  for (var i = 0u; i < SHAPES; i++) {
    // (The shape the burst looks at is held lit for its cut.)
    let held_shape = state.burst_hold > 0.5 && view > 4.5 && view < 5.5 && i == u32(state.burst_a);
    let lit = select(state.shape_look[i].y * fade, state.shape_look[i].y, held_shape);
    state.shape_look[i].y = lit;
    if (lit < 0.02) {
      continue;
    }
    place(i, th);
    let at = project(i);
    if (at.z >= 0.0) {
      state.hit[hits] = at;
      hits++;
    }
  }
  state.hit_n = hits;
}
