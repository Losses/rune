// Once per frame: the music moves the sphere as hands move a real one, in one of two ways. Free
// (see below): each ring always tumbling on its own, a push setting it tumbling a new way, surging
// round. Or as pendulums: each ring a body hung so it rests in its place: pushed, it swings out and
// comes back, slowed by friction; pushed hard, it goes over the top and on round a whole turn to
// its place again, a little past it and back. A push on one nudges its neighbours a moment later. A
// bar nudges one ring; a phrase of four bars flings one round a whole turn, each ring in its turn,
// each time the other way; a section flings three of them, one after another from the inside out,
// each the other way from the one before. Nothing steps with the beat. The gem at the heart rolls a
// whole turn each phrase. On the beats, the rings or (taking turns) a batch of shapes light and
// fade. The ecliptic precesses steadily. The core breathes with the music. A section cuts to
// another shot. Then the shapes are placed, projected and sorted into the screen's tiles.

// ---- Colour ----
// Every colour in the picture comes from one: the theme colour, the player's own (which follows
// the system, the listener's choice or the cover) unless the setting says to use its own. Only its hue is
// kept, and how colourful it is (held down, so it is never loud at night); every colour is made
// from them in OKLab, where a step in lightness or hue looks as big as it is: lines near white,
// the light warmer and fuller, the space round it all but black, a little turn of the hue here
// and there and nothing more.

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
// else of the setting.
fn theme() -> vec2f {
  let follow = params.follow == 1u && frame.accent.a > 0.5;
  let lab = oklab(select(params.theme.rgb, frame.accent.rgb, follow));
  return vec2f(atan2(lab.z, lab.y), min(length(lab.yz), 0.12) * params.chroma);
}

// The palette, by number: 0 the lines; 1 the core's light; 2 the rings' haze; 3, 4 the nebula's
// two shades (its hue turned a little either way); 5 deep space; 6, 7 the stars' two tints;
// 8 bronze; 9 the faint light of the stars; 10, 11 what space mirrors (deep, and towards the
// nebula); 12 glass; 13 the little star core's edge, fuller.
fn set_palette() {
  let th = theme();
  let h = th.x;
  let c = th.y;
  state.palette[0] = tint(0.92, c * 0.45, h);
  state.palette[1] = tint(0.8, c, h);
  state.palette[2] = tint(0.78, c * 0.9, h);
  state.palette[3] = vec4f(oklch(0.33, c * 0.6, h + 0.25), 0.0);
  state.palette[4] = vec4f(oklch(0.35, c * 0.5, h - 0.25), 0.0);
  state.palette[5] = vec4f(oklch(0.13, c * 0.25, h), 0.0);
  state.palette[6] = tint(0.93, c * 0.15, h - 0.5);
  state.palette[7] = tint(0.93, c * 0.25, h + 0.5);
  state.palette[8] = vec4f(oklch(0.62, c * 0.8, h), 0.0);
  state.palette[9] = tint(0.85, c * 0.3, h + 0.7);
  state.palette[10] = vec4f(oklch(0.17, c * 0.3, h), 0.0);
  state.palette[11] = vec4f(oklch(0.42, c * 0.5, h + 0.25), 0.0);
  state.palette[12] = tint(0.78, c * 0.9, h);
  state.palette[13] = tint(0.68, c * 1.2, h + 0.25);
  // Each ring the lines' colour, its hue turned a few degrees its own way.
  for (var i = 0u; i < RINGS; i++) {
    state.ring_tint[i] = tint(0.92, c * 0.45, h + (f32(i) - 2.0) * 0.07);
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

// The rings: outer radius, width, half thickness and ticks (0 none), by number: 0 the horizon,
// 1 the meridian, 2 the ecliptic, 3 the sun's ring, 4 an outer ring round them all (the moon's
// ring, once the innermost, sat too close to the core).
fn ring_size(i: u32) -> vec4f {
  var sizes = array<vec4f, 5>(vec4f(3.5, 0.22, 0.035, 24.0), vec4f(2.85, 0.15, 0.04, 36.0), vec4f(2.2, 0.24, 0.03, 72.0), vec4f(1.6, 0.08, 0.025, 12.0), vec4f(4.2, 0.12, 0.025, 0.0));
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
  if (params.motion == 0u) {
    retumble(k, way, across);
    surge(k, 6.2831853);
  } else {
    whirl(k, way, across);
  }
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
        if (params.motion == 0u) {
          retumble(i, w.x, w.w);
          surge(i, 6.2831853);
        } else {
          whirl(i, w.x, w.w);
        }
      } else if (params.motion == 0u) {
        surge(i, w.x);
      } else {
        push(i, w.x);
      }
    }
  }
  if (params.motion == 0u) {
    spin_ring(i, dt);
    return;
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

// A ring's frame from its axis and a direction in its plane (where its ticks start).
fn ring_frame(i: u32, axis: vec3f, along: vec3f) {
  let size = ring_size(i);
  let z = normalize(axis);
  let x = normalize(along - z * dot(along, z));
  let y = cross(z, x);
  state.ring_x[i] = vec4f(x, size.x);
  state.ring_y[i] = vec4f(y, size.y);
  state.ring_z[i] = vec4f(z, size.z);
  state.ring_look[i] = vec4f(size.w, 0.0, 0.0, 0.0);
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

// The rings now, from how each is moving; each also drifts a little on its own, as a thing
// floating does; the ecliptic precesses about the pole.
fn set_rings(t: f32) {
  for (var i = 0u; i < RINGS; i++) {
    let m = state.ring_turn[i];
    let about = state.ring_about[i].xyz;
    var a = turn(state.ring_rest_a[i].xyz, about, m.x);
    var b = turn(state.ring_rest_b[i].xyz, about, m.x);
    let drift = 0.03 * sin(t * (0.23 + 0.07 * f32(i)) + f32(i) * 1.7);
    a = turn(a, normalize(b), drift);
    if (i == 2u) {
      a = turn(a, pole(), state.spin);
      b = turn(b, pole(), state.spin);
    }
    ring_frame(i, a, b);
  }
}

// ---- The gem (core look 3) ----
// A ball of crystal cut all over in facets, cut by hand: laid out as a geodesic ball is (twelve
// pentagons and eighty hexagons: the facets' normals are the points of an icosahedron with each
// edge cut in three, and the middles of its faces), but each tipped a little its own way and cut
// a little deeper or shallower, so no two are quite alike.
// Drawn in lines: eighty triangles, as a crystal is cut. The corners of a geodesic ball (an
// icosahedron with each edge halved), each moved a little by hand along the ball and in or out,
// and a facet through every three that make a face: each its outward normal and how far it lies
// from the middle (w).
fn gem_facet_few(k: u32) -> vec4f {
  var planes = array<vec4f, 80>(
    vec4f(-0.28120, 0.93519, 0.21527, 0.90920),
    vec4f(0.32149, 0.92761, 0.19021, 0.91200),
    vec4f(-0.05288, 0.76212, 0.64527, 0.90214),
    vec4f(-0.07019, 0.91762, 0.39120, 0.90339),
    vec4f(-0.27341, 0.95234, -0.13527, 0.91142),
    vec4f(0.24492, 0.95475, -0.16874, 0.92199),
    vec4f(-0.02022, 0.81919, -0.57317, 0.90525),
    vec4f(-0.01200, 0.94655, -0.32233, 0.90293),
    vec4f(-0.58693, 0.74809, 0.30964, 0.92550),
    vec4f(-0.29509, 0.57041, 0.76652, 0.90470),
    vec4f(-0.70453, 0.34320, 0.62116, 0.93357),
    vec4f(-0.55556, 0.55447, 0.61961, 0.91731),
    vec4f(-0.59581, 0.74067, -0.31051, 0.91128),
    vec4f(-0.33558, 0.63151, -0.69899, 0.90208),
    vec4f(-0.77067, 0.27545, -0.57463, 0.92869),
    vec4f(-0.54149, 0.61544, -0.57274, 0.90072),
    vec4f(-0.80885, 0.58788, -0.01285, 0.90948),
    vec4f(-0.95940, 0.16474, -0.22895, 0.91041),
    vec4f(-0.96568, 0.17679, 0.19031, 0.90723),
    vec4f(-0.95115, 0.30872, -0.00110, 0.89167),
    vec4f(0.54171, 0.79488, 0.27332, 0.89794),
    vec4f(0.30216, 0.60964, 0.73283, 0.90702),
    vec4f(0.74171, 0.33102, 0.58334, 0.91637),
    vec4f(0.53566, 0.62226, 0.57084, 0.89770),
    vec4f(0.60120, 0.73771, -0.30715, 0.91467),
    vec4f(0.40658, 0.59689, -0.69168, 0.90666),
    vec4f(0.72858, 0.19824, -0.65565, 0.91044),
    vec4f(0.53357, 0.57942, -0.61610, 0.89878),
    vec4f(0.74614, 0.66551, -0.01927, 0.90325),
    vec4f(0.94661, 0.12519, -0.29707, 0.93424),
    vec4f(0.93893, 0.23788, 0.24863, 0.92133),
    vec4f(0.93499, 0.35352, 0.02863, 0.90901),
    vec4f(-0.30218, -0.93594, 0.18082, 0.91425),
    vec4f(0.20273, -0.96184, 0.18373, 0.92301),
    vec4f(-0.06226, -0.78075, 0.62174, 0.93054),
    vec4f(-0.03728, -0.94977, 0.31073, 0.91327),
    vec4f(-0.29224, -0.92816, -0.23047, 0.92492),
    vec4f(0.27043, -0.94821, -0.16664, 0.92335),
    vec4f(-0.08357, -0.73782, -0.66980, 0.92347),
    vec4f(-0.06866, -0.93178, -0.35646, 0.92690),
    vec4f(-0.56012, -0.77109, 0.30280, 0.91046),
    vec4f(-0.34432, -0.62706, 0.69875, 0.92127),
    vec4f(-0.78101, -0.28393, 0.55624, 0.92293),
    vec4f(-0.56505, -0.63219, 0.53014, 0.90718),
    vec4f(-0.51632, -0.77436, -0.36577, 0.92608),
    vec4f(-0.27122, -0.62456, -0.73237, 0.91302),
    vec4f(-0.72618, -0.31870, -0.60918, 0.92564),
    vec4f(-0.50274, -0.61747, -0.60497, 0.90975),
    vec4f(-0.76207, -0.64742, -0.01019, 0.93405),
    vec4f(-0.94981, -0.20244, -0.23848, 0.91731),
    vec4f(-0.96033, -0.12973, 0.24687, 0.91419),
    vec4f(-0.94680, -0.31773, -0.05117, 0.91556),
    vec4f(0.62268, -0.68765, 0.37337, 0.91442),
    vec4f(0.22882, -0.58650, 0.77695, 0.93344),
    vec4f(0.73319, -0.34629, 0.58525, 0.91115),
    vec4f(0.61940, -0.56285, 0.54731, 0.90264),
    vec4f(0.57519, -0.76031, -0.30179, 0.90926),
    vec4f(0.31386, -0.56505, -0.76302, 0.92216),
    vec4f(0.80409, -0.29178, -0.51798, 0.92601),
    vec4f(0.57050, -0.60814, -0.55200, 0.91042),
    vec4f(0.78386, -0.62058, -0.02102, 0.91381),
    vec4f(0.91542, -0.24868, -0.31650, 0.92746),
    vec4f(0.92776, -0.21528, 0.30481, 0.93997),
    vec4f(0.89533, -0.44309, -0.04545, 0.92115),
    vec4f(0.08862, -0.30509, 0.94819, 0.92225),
    vec4f(0.19204, 0.25758, 0.94698, 0.91091),
    vec4f(0.62133, 0.00318, 0.78354, 0.91197),
    vec4f(0.32586, 0.04116, 0.94452, 0.90630),
    vec4f(-0.20342, -0.29324, 0.93415, 0.91878),
    vec4f(-0.20511, 0.29648, 0.93275, 0.91067),
    vec4f(-0.60060, 0.00065, 0.79955, 0.92248),
    vec4f(-0.39015, -0.00487, 0.92074, 0.90901),
    vec4f(0.18979, -0.25236, -0.94884, 0.91354),
    vec4f(0.23982, 0.25499, -0.93673, 0.92996),
    vec4f(0.69648, 0.09682, -0.71101, 0.91136),
    vec4f(0.35592, 0.02968, -0.93404, 0.91639),
    vec4f(-0.10708, -0.26181, -0.95916, 0.91969),
    vec4f(-0.19030, 0.28130, -0.94056, 0.93019),
    vec4f(-0.65120, -0.03902, -0.75790, 0.93238),
    vec4f(-0.30622, 0.00415, -0.95195, 0.92487)
  );
  return planes[k];
}

fn gem_facet(k: u32) -> vec3f {
  var normals = array<vec3f, 92>(
    vec3f(-0.52573, 0.85065, 0.00000),
    vec3f(0.52573, 0.85065, 0.00000),
    vec3f(-0.52573, -0.85065, 0.00000),
    vec3f(0.52573, -0.85065, 0.00000),
    vec3f(0.00000, -0.52573, 0.85065),
    vec3f(0.00000, 0.52573, 0.85065),
    vec3f(0.00000, -0.52573, -0.85065),
    vec3f(0.00000, 0.52573, -0.85065),
    vec3f(0.85065, 0.00000, -0.52573),
    vec3f(0.85065, 0.00000, 0.52573),
    vec3f(-0.85065, 0.00000, -0.52573),
    vec3f(-0.85065, 0.00000, 0.52573),
    vec3f(-0.20177, 0.97943, 0.00000),
    vec3f(0.20177, 0.97943, 0.00000),
    vec3f(-0.40355, 0.85473, 0.32648),
    vec3f(-0.20177, 0.73003, 0.65295),
    vec3f(-0.40355, 0.85473, -0.32648),
    vec3f(-0.20177, 0.73003, -0.65295),
    vec3f(-0.73003, 0.65295, -0.20177),
    vec3f(-0.85473, 0.32648, -0.40355),
    vec3f(-0.73003, 0.65295, 0.20177),
    vec3f(-0.85473, 0.32648, 0.40355),
    vec3f(0.40355, 0.85473, 0.32648),
    vec3f(0.20177, 0.73003, 0.65295),
    vec3f(0.40355, 0.85473, -0.32648),
    vec3f(0.20177, 0.73003, -0.65295),
    vec3f(0.73003, 0.65295, -0.20177),
    vec3f(0.85473, 0.32648, -0.40355),
    vec3f(0.73003, 0.65295, 0.20177),
    vec3f(0.85473, 0.32648, 0.40355),
    vec3f(-0.20177, -0.97943, 0.00000),
    vec3f(0.20177, -0.97943, 0.00000),
    vec3f(-0.40355, -0.85473, 0.32648),
    vec3f(-0.20177, -0.73003, 0.65295),
    vec3f(-0.40355, -0.85473, -0.32648),
    vec3f(-0.20177, -0.73003, -0.65295),
    vec3f(-0.73003, -0.65295, -0.20177),
    vec3f(-0.85473, -0.32648, -0.40355),
    vec3f(-0.73003, -0.65295, 0.20177),
    vec3f(-0.85473, -0.32648, 0.40355),
    vec3f(0.40355, -0.85473, 0.32648),
    vec3f(0.20177, -0.73003, 0.65295),
    vec3f(0.40355, -0.85473, -0.32648),
    vec3f(0.20177, -0.73003, -0.65295),
    vec3f(0.73003, -0.65295, -0.20177),
    vec3f(0.85473, -0.32648, -0.40355),
    vec3f(0.73003, -0.65295, 0.20177),
    vec3f(0.85473, -0.32648, 0.40355),
    vec3f(0.00000, -0.20177, 0.97943),
    vec3f(0.00000, 0.20177, 0.97943),
    vec3f(0.32648, -0.40355, 0.85473),
    vec3f(0.65295, -0.20177, 0.73003),
    vec3f(-0.32648, -0.40355, 0.85473),
    vec3f(-0.65295, -0.20177, 0.73003),
    vec3f(0.32648, 0.40355, 0.85473),
    vec3f(0.65295, 0.20177, 0.73003),
    vec3f(-0.32648, 0.40355, 0.85473),
    vec3f(-0.65295, 0.20177, 0.73003),
    vec3f(0.00000, -0.20177, -0.97943),
    vec3f(0.00000, 0.20177, -0.97943),
    vec3f(0.32648, -0.40355, -0.85473),
    vec3f(0.65295, -0.20177, -0.73003),
    vec3f(-0.32648, -0.40355, -0.85473),
    vec3f(-0.65295, -0.20177, -0.73003),
    vec3f(0.32648, 0.40355, -0.85473),
    vec3f(0.65295, 0.20177, -0.73003),
    vec3f(-0.32648, 0.40355, -0.85473),
    vec3f(-0.65295, 0.20177, -0.73003),
    vec3f(0.97943, 0.00000, -0.20177),
    vec3f(0.97943, 0.00000, 0.20177),
    vec3f(-0.97943, 0.00000, -0.20177),
    vec3f(-0.97943, 0.00000, 0.20177),
    vec3f(0.00000, 0.93417, 0.35682),
    vec3f(0.00000, 0.93417, -0.35682),
    vec3f(-0.57735, 0.57735, 0.57735),
    vec3f(-0.57735, 0.57735, -0.57735),
    vec3f(-0.93417, 0.35682, 0.00000),
    vec3f(0.57735, 0.57735, 0.57735),
    vec3f(0.57735, 0.57735, -0.57735),
    vec3f(0.93417, 0.35682, 0.00000),
    vec3f(0.00000, -0.93417, 0.35682),
    vec3f(0.00000, -0.93417, -0.35682),
    vec3f(-0.57735, -0.57735, 0.57735),
    vec3f(-0.57735, -0.57735, -0.57735),
    vec3f(-0.93417, -0.35682, 0.00000),
    vec3f(0.57735, -0.57735, 0.57735),
    vec3f(0.57735, -0.57735, -0.57735),
    vec3f(0.93417, -0.35682, 0.00000),
    vec3f(0.35682, 0.00000, 0.93417),
    vec3f(-0.35682, 0.00000, 0.93417),
    vec3f(0.35682, 0.00000, -0.93417),
    vec3f(-0.35682, 0.00000, -0.93417)
  );
  return normals[k];
}

// The gem turns as a free ring does: slowly all the time about its axis (a little quicker on
// each kick, so its facets flash with the beat), and at each phrase it rolls a whole turn about
// a new axis, surging round and easing back to its pace.
fn spin_gem(dt: f32) {
  var m = state.gem_roll;
  let time = params.whirl_time;
  let coast = G_COAST * time;
  let drive = G_DRIVE * time;
  let pace = (0.25 + 1.5 * state.kick) * params.flow;
  let n = 4;
  let h = dt / f32(n);
  for (var k = 0; k < n; k++) {
    m.y += (m.z + (pace - m.y) / coast) * h;
    m.x += m.y * h;
    m.z *= exp(-h / drive);
  }
  state.gem_roll = m;
}

// The gem rolls a whole turn, `way` (1 or -1) round about the line on the screen at angle
// `across`: where it is now becomes where it turns from, its speed kept.
fn roll_gem(way: f32, across: f32) {
  let m = state.gem_roll;
  let about = state.gem_about.xyz;
  state.gem_rest_a = vec4f(turn(state.gem_rest_a.xyz, about, m.x), 0.0);
  state.gem_rest_b = vec4f(turn(state.gem_rest_b.xyz, about, m.x), 0.0);
  state.gem_about = vec4f((state.cam_right * cos(across) + state.cam_up * sin(across)) * way, 0.0);
  let time = params.whirl_time;
  state.gem_roll = vec4f(0.0, abs(m.y), m.z + 6.2831853 / (G_DRIVE * time * G_COAST * time), 0.0);
}

// The gem's facets as it is turned now.
fn set_gem(t: f32) {
  let m = state.gem_roll;
  let about = state.gem_about.xyz;
  let ey = normalize(turn(state.gem_rest_a.xyz, about, m.x));
  let b = turn(state.gem_rest_b.xyz, about, m.x);
  let ex = normalize(b - ey * dot(b, ey));
  let ez = cross(ex, ey);
  let few = params.core_look == 4u;
  state.gem_count = select(FACETS, 80u, few);
  for (var k = 0u; k < state.gem_count; k++) {
    let id = vec2f(f32(k), 9.1);
    let lean = (vec3f(hash(id), hash(id + 0.3), hash(id + 0.7)) - 0.5) * 0.25;
    var f = vec4f(normalize(gem_facet(k) + lean), 1.0 - 0.05 * hash(id + 1.1));
    if (few) {
      f = gem_facet_few(k);
    }
    state.gem[k] = vec4f(normalize(ex * f.x + ey * f.y + ez * f.z), f.w);
  }
}

// ---- Accents (look 2) ----
// Drawn as lines, the rings rest as faint hairlines and an accent lights one at once, thicker
// and bright, fading back over a moment (a strong one lights two); most times it also takes a
// new tilt as it lights, so the accent brings a new shape, not only light.

// Ring i lit as strongly as `strength`. Drawn in marks, its dots switch on, one after another
// both ways from somewhere new, until `light_reach` of the ring is lit; then they dim together.
fn light_ring(i: u32, strength: f32, n: u32) {
  state.ring_lit[i].x = max(state.ring_lit[i].x, strength);
  if (params.look == 3u) {
    state.ring_on[i] = vec4f(random(60u + n), 0.0, strength, 0.5 * params.light_reach);
  }
  if (params.look >= 2u && params.motion == 0u && random(20u + n) < params.renew) {
    let up = random(30u + n) * 2.0 - 1.0;
    let round = random(40u + n) * 6.2831853;
    let a = vec3f(sqrt(1.0 - up * up) * cos(round), up, sqrt(1.0 - up * up) * sin(round));
    let b = normalize(cross(a, select(vec3f(0.0, 1.0, 0.0), vec3f(1.0, 0.0, 0.0), abs(up) > 0.9)));
    state.ring_rest_a[i] = vec4f(a, 0.0);
    state.ring_rest_b[i] = vec4f(b, 0.0);
    state.ring_about[i] = vec4f(cross(a, b), 0.0);
    state.ring_turn[i].x = 0.0;
  }
}

// An accent as strong as `strength` lights whichever of the two is lighting now: the next ring
// in an order that leaps about, and on a strong one another with it; or a batch of the shapes.
fn light(strength: f32) {
  state.lits += 1.0;
  var order = array<u32, 5>(2u, 0u, 3u, 1u, 4u);
  let n = u32(state.lits);
  if (state.layer < 0.5) {
    light_ring(order[n % 5u], strength, 0u);
    if (strength > 0.85) {
      light_ring(order[(n + 2u) % 5u], strength * 0.7, 1u);
    }
    return;
  }
  // A batch of the shapes, the next batch in turn, each made anew as it lights
  // (somewhere else, another shape): unlit they are not there, so no one sees them change.
  // The batch before gives way to it, dimming at once to a third and fading on from there.
  let batch = n % BATCHES;
  let tip = (vec3f(random(50u), random(51u), random(52u)) - 0.5) * 1.2;
  state.batch_axis[batch] = vec4f(normalize(-state.cam_fwd + tip), 0.0);
  // How many: never the same, more on a stronger beat. How big: as they are close up, up to
  // twice that seen from far off, so they look much the same size in every shot.
  let most = f32(IN_BATCH) * params.shapes;
  let count = clamp(round(most * mix(0.35, 1.0, random(53u)) * mix(0.7, 1.0, strength)), 2.0, f32(IN_BATCH));
  let far = clamp(length(state.cam_pos - state.centre) / 8.0, 1.0, 2.0);
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

// ---- Free (motion 0) ----
// Each ring on its own, always turning: slowly and evenly with the tempo about a line across its
// own plane (so it tumbles), the small ones quicker. A push sends it surging further round,
// easing back to its own pace after; a push of its own also sets it tumbling about a new line,
// and rings pushed together are set tumbling every one a different way.

// How fast ring i turns of itself, in radians a second: degrees a beat, with the tempo.
fn own_pace(i: u32) -> f32 {
  var rate = array<f32, 5>(3.0, 4.5, 6.0, 9.0, 13.0);
  return radians(rate[i]) * params.ring_spin * state.tempo / 60.0;
}

// How long a surge's push lasts, and how long it takes the ring to ease back to its pace, in
// proportion to the ring's time.
const G_DRIVE: f32 = 0.1;
const G_COAST: f32 = 0.45;

// Ring i sent `angle` further round (negative: held back, or turned back a little).
fn surge(i: u32, angle: f32) {
  let time = ring_time(i);
  state.ring_turn[i].z += angle / (G_DRIVE * time * G_COAST * time);
}

// Ring i set tumbling `way` (1 or -1) about the line across its plane nearest the line on the
// screen at angle `across`: where it is now becomes where it turns from, its speed kept.
fn retumble(i: u32, way: f32, across: f32) {
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
  state.ring_about[i] = vec4f(normalize(onplane) * way, 0.0);
  state.ring_turn[i] = vec4f(0.0, abs(m.y), m.z, m.w);
}

// Ring i turning for dt: its speed driven by any push, easing back to its own pace; and the
// slow turn of its ticks round its own plane.
fn spin_ring(i: u32, dt: f32) {
  var m = state.ring_turn[i];
  let time = ring_time(i);
  let coast = G_COAST * time;
  let drive = G_DRIVE * time;
  let pace = own_pace(i);
  let n = 4;
  let h = dt / f32(n);
  for (var k = 0; k < n; k++) {
    m.y += (m.z + (pace - m.y) / coast) * h;
    m.x += m.y * h;
    m.z *= exp(-h / drive);
  }
  m.w += pace * 0.5 * dt;
  state.ring_turn[i] = m;
}

// The rings now, each from where it turns from, turned so far about its line; each drifting a
// little, as a thing floating does.
fn set_free(t: f32) {
  for (var i = 0u; i < RINGS; i++) {
    let m = state.ring_turn[i];
    let about = state.ring_about[i].xyz;
    var a = turn(state.ring_rest_a[i].xyz, about, m.x);
    var b = turn(state.ring_rest_b[i].xyz, about, m.x);
    a = turn(a, normalize(b), 0.03 * sin(t * (0.23 + 0.07 * f32(i)) + f32(i) * 1.7));
    // Its ticks turn round its own plane.
    ring_frame(i, a, turn(b, normalize(a), m.w));
  }
}

// Shape i: where it hangs and how it lies, made anew each time its batch is lit (from how many
// times that is). A polygon of three to six sides, hanging just outside the outermost ring; the
// shapes of a batch (how many, and how big for the camera's distance, set as it lit) spaced
// unevenly round a band about the batch's axis (facing the camera as the batch lit, tipped a
// little), so together they ring the sphere round as it is seen, none in front of it; each
// facing the middle more or less, turning slowly where it is.
fn place(i: u32) {
  let batch = i % BATCHES;
  let place_in = i / BATCHES;
  let look = state.batch_look[batch];
  let many = max(u32(look.x), 1u);
  let present = f32(place_in < many);
  let id = vec2f(f32(i), 3.7 + state.shape_look[i].z * 17.31);
  let band_id = vec2f(f32(batch), 5.3 + state.shape_look[i].z * 17.31);
  let t = frame.time;
  // The batch's band, its shapes round it, unevenly spaced, some well above or below it.
  let axis = state.batch_axis[batch].xyz;
  let e1 = normalize(cross(axis, select(vec3f(0.0, 1.0, 0.0), vec3f(1.0, 0.0, 0.0), abs(axis.y) > 0.9)));
  let e2 = cross(axis, e1);
  let round_at = hash(band_id + 0.3) * 6.2831853 + (f32(place_in) + 0.85 * (hash(id + 0.4) - 0.5)) * 6.2831853 / f32(many);
  let lift = (hash(id + 0.2) - 0.5) * 1.0;
  let out = normalize((e1 * cos(round_at) + e2 * sin(round_at)) * cos(lift) + axis * sin(lift));
  let r = mix(4.6, 6.1, hash(id + 0.1));
  let size = mix(0.25, 0.95, pow(hash(id + 0.5), 1.5)) * look.y * params.shape_size;
  state.shape_pos[i] = vec4f(state.centre + out * r, size * present);
  // Facing the middle, tipped (some well over), turning slowly in its own plane.
  let n = normalize(out + (vec3f(hash(id + 2.1), hash(id + 2.2), hash(id + 2.3)) - 0.5) * 1.1);
  let u0 = normalize(cross(n, select(vec3f(0.0, 1.0, 0.0), vec3f(1.0, 0.0, 0.0), abs(n.y) > 0.9)));
  let angle = t * mix(0.12, 0.3, hash(id + 2.7)) * select(-1.0, 1.0, hash(id + 0.3) < 0.5) + hash(id + 2.8) * 6.2831853;
  let sides = 3.0 + floor(hash(id + 2.9) * 4.0);
  state.shape_n[i] = vec4f(n, sides);
  state.shape_u[i] = vec4f(turn(u0, n, angle), 0.0);
  // The lines' colour, a little lighter or darker, its hue turned up to ten degrees either way.
  let th = theme();
  state.shape_tint[i] = tint(0.92 + 0.05 * (hash(id + 3.1) - 0.5), th.y * 0.5, th.x + (hash(id + 3.2) - 0.5) * 0.35);
}

// Where shape i is on the screen, how far it reaches, and which tiles list it.
fn project(i: u32) {
  let p = state.shape_pos[i];
  if (p.w <= 0.0) {
    state.shape_scr[i] = vec4f(0.0, 0.0, -1.0, -1.0);
    return;
  }
  let v = p.xyz - state.cam_pos;
  let zc = dot(v, state.cam_fwd);
  // (Not one right by the lens: it would be a blur across the picture.)
  if (zc < 3.0) {
    state.shape_scr[i] = vec4f(0.0, 0.0, -1.0, -1.0);
    return;
  }
  let aspect = frame.aspect;
  let c = state.focal * vec2f(dot(v, state.cam_right), dot(v, state.cam_up)) / zc;
  // As far as its corners, and a few pixels for its line and glow.
  let rad = state.focal * p.w * 1.05 / zc + 5.0 / frame.resolution.y;
  state.shape_scr[i] = vec4f(c, rad, zc);
  // (Unlit, it is not there: no tile lists it.)
  if (abs(c.x) - rad > aspect * 0.5 || abs(c.y) - rad > 0.5 || state.shape_look[i].y < 0.02) {
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

fn update() {
  let dt = min(frame.dt, 0.05);
  let t = frame.time;
  if (frame.first == 1u) {
    state.moment = 0.4;
    state.tempo = 100.0;
    state.core_size = 1.0;
    rest_rings();
    state.spec_ring = 4.0;
    state.gem_rest_a = vec4f(turn(vec3f(0.0, 1.0, 0.0), vec3f(0.0, 0.0, 1.0), 0.45), 0.0);
    state.gem_rest_b = vec4f(1.0, 0.0, 0.0, 0.0);
    state.gem_about = state.gem_rest_a;
  }
  let chart = music.has_chart == 1u;
  let now = select(clamp((music.energy - 0.6) / 1.0, 0.0, 1.0), music.loudness, chart);
  state.moment += (now - state.moment) * (1.0 - exp(-dt / 0.8));
  let heard = vec3f(music.bass, music.mids, music.treble);
  state.levels = select(state.levels + (heard - state.levels) * (1.0 - exp(-dt / 0.2)), state.levels + (heard - state.levels) * (1.0 - exp(-dt / 0.04)), heard > state.levels);

  // Rings move by the bar, the phrase and the section, never step by step on the beat: a bar
  // nudges one ring, each in its turn; a phrase of four bars flings one round a whole turn, each
  // in its turn, its neighbours nudged after; a section flings three, from the inside out.
  // Without a beat, a bar every four seconds; without sections, every four phrases. The kick
  // swells the core.
  let sure = music.bpm > 0.0 && music.beat_confidence >= params.conf;
  let beat = (music.beats > 0u && sure) || (!sure && floor(t) != floor(t - dt));
  if (beat) {
    state.kick = 1.0;
    state.core_vel += params.core_kick;
  }
  var section = music.sections > 0u && frame.first == 0u;
  let barred = (music.bars > 0u && sure) || (!sure && floor(t / 4.0) != floor((t - dt) / 4.0));
  if (barred) {
    state.bars += 1.0;
    let bar = u32(state.bars);
    if (bar % 4u == 0u) {
      state.phrases += 1.0;
      if (!chart && u32(state.phrases) % 4u == 0u) {
        section = true;
      }
      // Another ring (never the ecliptic, which shows the spectrum in its ticks, nor the one
      // before) becomes the line of the spectrum.
      var others = array<u32, 4>(0u, 1u, 3u, 4u);
      var pick = others[u32(random(60u) * 4.0) % 4u];
      if (f32(pick) == state.spec_ring) {
        pick = others[(u32(random(60u) * 4.0) + 1u + u32(random(61u) * 3.0)) % 4u];
      }
      state.spec_ring = f32(pick);
      // The gem rolls a whole turn, the other way from the last.
      roll_gem(select(-1.0, 1.0, u32(state.phrases) % 2u == 1u), random(7u) * 6.2831853);
      if (!section) {
        // The rings in an order that leaps about: the moon's, the ecliptic, the horizon, the
        // sun's, the meridian.
        var order = array<u32, 5>(4u, 2u, 0u, 3u, 1u);
        let way = select(-1.0, 1.0, u32(state.phrases) % 2u == 0u);
        whirl_one(order[u32(state.phrases) % 5u], way, random(4u) * 6.2831853);
      }
    } else {
      let k = (bar * 2u + 1u) % RINGS;
      if (params.motion == 0u) {
        // Set tumbling a new way, and a little further round.
        retumble(k, select(-1.0, 1.0, bar % 2u == 0u), random(6u) * 3.1415927);
        surge(k, radians(params.sway) * 6.0);
      } else {
        push(k, radians(params.sway) * 1.5 * select(-1.0, 1.0, bar % 2u == 0u));
      }
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
  if (beat) {
    state.beat_in_bar = select(state.beat_in_bar + 1.0, 0.0, barred);
  }
  // The rings and the shapes take turns to light, every `layer_bars` bars.
  if (barred && u32(state.bars) % u32(max(params.layer_bars, 1)) == 0u) {
    state.layer = 1.0 - state.layer;
  }
  state.layer_mix += (state.layer - state.layer_mix) * (1.0 - exp(-dt / 0.4));
  state.kick *= exp(-dt / 0.25);
  for (var i = 0u; i < RINGS; i++) {
    move_ring(i, dt);
  }
  spin_gem(dt);

  // Accents fall on the beats: the bar's first beat strongest, its third next, the others least,
  // all of it stronger as the music is louder (so in a quiet passage only the strong beats
  // light). What is lit fades by the beat: by half in `lit_beats` beats.
  if (beat) {
    let at = u32(state.beat_in_bar) % 4u;
    let weight = select(select(0.5, 0.75, at == 2u), 1.0, at == 0u);
    let accent = weight * mix(0.55, 1.0, state.moment);
    if (accent > params.accent) {
      light(accent);
    }
  }
  let fade = exp(-dt * 0.693 / (params.lit_beats * 60.0 / max(state.tempo, 40.0)));
  let beat_time = 60.0 / max(state.tempo, 40.0);
  let on_fade = exp(-dt * 0.693 / (params.light_beats * beat_time));
  for (var i = 0u; i < RINGS; i++) {
    state.ring_lit[i].x *= fade;
    // The dots come on quickly at first and slower as they reach the last ones, and all dim
    // together.
    var on = state.ring_on[i];
    on.y += (on.w - on.y) * (1.0 - exp(-dt * 3.0 / max(params.light_spread * beat_time, 0.01)));
    on.z *= on_fade;
    state.ring_on[i] = on;
    // The ring of the spectrum eases out of its circle, the one before back into it.
    let to = f32(f32(i) == state.spec_ring);
    state.ring_spec[i].x += (to - state.ring_spec[i].x) * (1.0 - exp(-dt / 0.6));
  }
  for (var i = 0u; i < SHAPES; i++) {
    state.shape_look[i].y *= fade;
  }

  // The core: as big as the music is loud (with the bass), springing out on each kick and back.
  let big = 1.0 + 0.35 * state.moment + 0.12 * clamp(state.levels.x - 1.0, 0.0, 1.5);
  var size = state.core_size;
  var size_vel = state.core_vel;
  spring(&size, &size_vel, big, dt, 12.0, 0.35);
  state.core_size = size;
  state.core_vel = size_vel;

  // Between the pushes everything keeps going, each at its own pace, all of it quicker as the
  // music is louder: the ecliptic's precession, the camera, the core's surface
  // (with the treble) and rays (with the mids).
  let pace = params.flow * mix(0.35, 1.6, state.moment);
  state.spin += dt * 0.06 * pace;
  // The tempo, smoothed so a change of it (or of song) never jolts what turns with it; without
  // a tempo, as at 100 beats a minute.
  let bpm = select(100.0, music.bpm, music.bpm > 0.0 && sure);
  state.tempo += (bpm - state.tempo) * (1.0 - exp(-dt / 2.0));
  state.drift_cam += dt * params.drift * mix(0.6, 1.5, state.moment);
  state.core_flow += dt * (0.25 + 0.6 * clamp(state.levels.z, 0.0, 2.0)) * params.flow;
  state.core_rays += dt * (0.1 + 0.4 * clamp(state.levels.y, 0.0, 2.0)) * params.flow;
  // The character-art planet turns with the tempo, quicker as it is louder: a turn in about 24
  // beats.
  state.planet_turn += dt * 6.2831853 / (24.0 * 60.0 / max(state.tempo, 40.0)) * mix(0.6, 1.4, state.moment) * params.flow;
  // The sphere floats: rising and sinking a little, drifting.
  state.centre = vec3f(0.05 * sin(t * 0.31), 0.08 * sin(t * 0.45), 0.05 * cos(t * 0.27));

  // A section cuts to another shot; without sections, every so often.
  var cut = frame.first == 1u || section;
  if (!chart && t - state.last_view > params.shot_time) {
    cut = true;
  }
  if (cut) {
    var v = floor(random(1u) * 4.0);
    if (v == state.view && frame.first == 0u) {
      v = (v + 1.0 + floor(random(2u) * 3.0)) % 4.0;
    }
    state.view = v;
    state.side = select(-1.0, 1.0, random(3u) < 0.5);
    state.last_view = t;
    state.drift_cam = 0.0;
  }
  var view = state.view;
  if (params.view > 0u) {
    view = f32(params.view - 1u);
  }

  // The shot, about the sphere floating in space: 0 a little below, pushing in; 1 far off, the
  // sphere small among the stars, circling; 2 close, the rings crossing, circling; 3 from above,
  // circling. Each mirrored by its side; the camera moves slowly and evenly, never with the beat.
  let c = state.centre;
  let s = state.side;
  let held = state.drift_cam;
  let round = s * held;
  let up = vec3f(0.0, 1.0, 0.0);
  var cam: vec3f;
  var look: vec3f;
  var focal: f32;
  if (view < 0.5) {
    look = c + vec3f(0.0, 0.3, 0.0);
    cam = mix(c + vec3f(4.4 * s, -1.8, -5.6), look, 0.3 * (1.0 - exp(-held / 30.0)));
    focal = 0.9;
  } else if (view < 1.5) {
    cam = c + turn(vec3f(6.5 * s, 1.8, -9.4), up, round * 0.012);
    look = c;
    focal = 1.25;
  } else if (view < 2.5) {
    cam = c + turn(vec3f(2.6 * s, 1.0, -3.4), up, round * 0.02);
    look = c + turn(vec3f(-0.3 * s, -0.2, 0.0), up, round * 0.02);
    focal = 1.0;
  } else {
    cam = c + turn(vec3f(1.6 * s, 8.0, -4.2), up, round * 0.015);
    look = c;
    focal = 1.0;
  }
  // Further back with a longer lens, the same framing with less of the near side of each ring
  // swelling: concentric rings look concentric.
  cam = look + (cam - look) * params.lens;
  focal *= params.lens;
  let fwd = normalize(look - cam);
  let right = normalize(cross(up, fwd));
  state.cam_pos = cam;
  state.cam_fwd = fwd;
  state.cam_right = right;
  state.cam_up = cross(fwd, right);
  state.focal = focal;
  // Drawn in marks, what is bright fringes a little red and blue, as on a screen.
  if (params.look == 3u) {
    post.chromatic = 0.0022 * (1.0 + state.kick);
  }

  if (params.motion == 0u) {
    set_free(t);
  } else {
    set_rings(t);
  }
  set_palette();
  set_gem(t);
  for (var k = 0u; k < TILES; k++) {
    state.tile_n[k] = 0u;
  }
  for (var i = 0u; i < SHAPES; i++) {
    place(i);
    project(i);
  }
}
