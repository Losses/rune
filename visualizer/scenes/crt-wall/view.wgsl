// The wall as the camera sees it. The room is dark: one cool key light from high on the left
// throws the piles' shadows, and the screens light what stands beside them. The cabinets come in
// a few finishes (charcoal, cream, silver, gloss black, walnut, the odd red), with strips of
// brushed metal or speaker cloth; every screen shows one of three channels, and changes channel
// on its own beat in a flash: it brightens, cuts at the brightest and settles.

// Lines down a picture.
const LINES: f32 = 96.0;
// How long a screen brightens before it cuts.
const RISE: f32 = 0.06;

struct Ray {
  o: vec3f,
  d: vec3f,
}

fn forward() -> vec3f {
  return vec3f(sin(state.yaw) * cos(state.pitch), sin(state.pitch), cos(state.yaw) * cos(state.pitch));
}

fn camera_ray(px: Pixel) -> Ray {
  let f = forward();
  let right = vec3f(cos(state.yaw), 0.0, -sin(state.yaw));
  let up = cross(f, right);
  var ray: Ray;
  ray.o = state.cam;
  ray.d = normalize(f + (right * px.p.x + up * px.p.y) * 2.0 * state.lens);
  return ray;
}

// ---- Channels ----

// What a screen shows now: its channel (0 snow, 1 scope, 2 a cover, 3 off), how bright the flash
// of a change is, how hard the latest beat reached it, and its own number for the channel's
// details.
struct Screen {
  chan: u32,
  flash: f32,
  amt: f32,
  seed: f32,
}

// One channel by the weights of the three (no covers until the first have come).
fn weighted(u: f32) -> u32 {
  let w = max(vec3f(params.snow, params.scope, params.cover * f32(frame.covers > 0u)), vec3f(0.0));
  let x = u * max(w.x + w.y + w.z, 1e-3);
  return select(select(2u, 1u, x < w.x + w.y), 0u, x < w.x);
}

// The channel a set is on after its kk-th change. Sets near one another lean to the same channel
// (each section groups them anew), and in a quiet section some are off.
fn channel(s: Set, kk: f32) -> u32 {
  let region = floor(s.centre.xy / vec2f(1.8, 1.3));
  if (hash(s.id * 1.37 + vec2f(state.section, 4.4)) < state.dark) {
    return 3u;
  }
  if (hash(s.id + vec2f(kk * 1.7, 9.1)) < params.group) {
    return weighted(hash(region + vec2f(state.section * 7.1, 2.3)));
  }
  return weighted(hash(s.id * 2.11 + vec2f(kk, 6.6)));
}

fn screen_of(s: Set) -> Screen {
  var sc: Screen;
  // Each set hears the beat a little late, by its own share of a beat (in quarters).
  let lag = floor(hash(s.id + 1.3) * params.stagger * 4.0) * 0.25;
  let c = state.clock - lag;
  let sb = floor(c);
  // A set changes channel every 1/share beats, each on its own beat.
  let share = clamp(params.share, 0.01, 1.0);
  let h = hash(s.id + 2.9);
  let kk = floor(sb * share + h);
  sc.chan = channel(s, kk);
  sc.seed = hash(s.id + vec2f(kk * 0.61, 0.0)) * 97.0;
  // The flash: rising just before the change, cut at its brightest, then falling away. A change
  // that lands on the same channel is still a change of picture.
  let since = (c - ceil((kk - h) / share)) * state.beat_len;
  let until = (ceil((kk + 1.0 - h) / share) - c) * state.beat_len;
  let before = 1.0 - smoothstep(0.0, RISE, until);
  let after = exp(-since / max(params.flash_time, 0.01));
  sc.flash = max(before, after) * params.flash * state.sure;
  // Some sets jump on each beat, each by its own amount.
  let jolt = hash(s.id * 1.9 + vec2f(sb * 4.3, 2.7));
  let react = select(0.0, 0.6 + 0.8 * hash(s.id * 2.3 + vec2f(sb * 7.1, 5.5)), jolt < params.react);
  sc.amt = react * exp(-fract(c) * state.beat_len / max(params.kick_decay, 0.01)) * state.sure;
  return sc;
}

// The light a screen gives on average, for what it lights around it.
fn screen_glow(sc: Screen) -> vec3f {
  var c = vec3f(0.0);
  if (sc.chan == 0u) {
    c = state.phosphor * 0.3;
  } else if (sc.chan == 1u) {
    c = state.phosphor * 0.1;
  } else if (sc.chan == 2u && frame.covers > 0u) {
    // The cover's own average colour, from its smallest mip.
    let k = cover_cell(sc.seed);
    c = textureSampleLevel(covers, linear_clamp, (vec2f(f32(k % 8u), f32(k / 8u)) + 0.5) / 8.0, 7.0).rgb * 0.35 * params.cover_light;
  }
  return (c + state.phosphor * 0.6 * sc.flash) * params.brightness;
}

// ---- Pictures; s is 0..1 across the picture (y up), asp its width over its height, foot a
// pixel's size in picture heights ----

// The phosphor's colour for a light v: its own colour, whitening at the brightest.
fn phosphor(v: f32) -> vec3f {
  return state.phosphor * v + vec3f(max(v - 0.85, 0.0) * 0.7);
}

// Snow: the noise of no station, torn sideways by the beat, a bright bar slowly rolling up.
fn snow(s: vec2f, asp: f32, seed: f32, amt: f32, fade: f32) -> f32 {
  let t = frame.time;
  let band = floor(s.y * 24.0);
  let tear = (hash(vec2f(band, floor(t * 24.0) + seed)) - 0.5) * 0.22 * amt;
  let g = floor(vec2f((s.x + tear) * asp * LINES * 2.0, s.y * LINES));
  let n = hash(g + vec2f(floor(t * 30.0) * 13.17, seed * 7.3));
  let bar = 0.18 * smoothstep(0.82, 1.0, sin(s.y * 3.0 - t * 1.7 + seed));
  return mix(n, 0.5, fade * 0.4) * 0.3 + bar * 0.7;
}

// A scope: the beam's trace (from the scope buffer) on a graticule. Mode by the seed: the stereo
// picture in a square in the middle, or the wave against time across the screen.
fn scope(s: vec2f, asp: f32, seed: f32, fade: f32) -> f32 {
  var v = 0.0;
  if (fract(seed) < 0.5) {
    let q = vec2f((s.x - 0.5) * asp, s.y - 0.5) * 2.1;
    if (all(abs(q) < vec2f(1.0))) {
      v = textureSampleLevel(scope_buf, linear_clamp, vec2f(0.25 + q.x * 0.25, 0.5 - q.y * 0.5), 0.0).r;
    }
  } else {
    v = textureSampleLevel(scope_buf, linear_clamp, vec2f(0.5 + clamp(s.x, 0.001, 0.999) * 0.5, 1.0 - s.y), 0.0).r;
  }
  // The graticule: ten by eight squares, faint, gone where a pixel is wider than a line.
  let g = abs(fract(s * vec2f(10.0, 8.0)) - 0.5);
  let line = smoothstep(0.47, 0.49, max(g.x, g.y)) * (1.0 - fade);
  return 1.3 * v + 0.05 * line;
}

// Which cell of the covers a screen shows: now and then the cover of the song playing (cell 0),
// else one of the library's; the playing one only when there is one.
fn cover_cell(seed: f32) -> u32 {
  let n = frame.covers;
  if (n <= 1u) {
    return 0u;
  }
  let now = fract(seed * 5.31) < params.now_playing;
  // The smallest mip holds one texel a cell: whether cell 0 has a cover at all.
  let playing = textureSampleLevel(covers, linear_clamp, vec2f(0.0625), 7.0).a > 0.5;
  if (now && playing) {
    return 0u;
  }
  return 1u + min(u32(fract(seed * 0.7310) * f32(n - 1u)), n - 2u);
}

// A cover, square in the middle of the picture, black either side as an old set shows a square
// picture; swelling a little on the beat. Close up, the tube's stripes of red, green and blue.
fn cover(s: vec2f, asp: f32, seed: f32, amt: f32, foot: f32) -> vec3f {
  if (frame.covers == 0u) {
    return vec3f(0.0);
  }
  let k = cover_cell(seed);
  let side = 0.94 * (1.0 + 0.04 * amt);
  let f = vec2f((s.x - 0.5) * asp, s.y - 0.5) / side + 0.5;
  if (any(f < vec2f(0.0)) || any(f > vec2f(1.0))) {
    return vec3f(0.0);
  }
  let uv = (vec2f(f32(k % 8u), f32(k / 8u)) + clamp(vec2f(f.x, 1.0 - f.y), vec2f(0.01), vec2f(0.99))) / 8.0;
  let lod = log2(max(foot / side * 128.0, 1.0));
  let c = textureSampleLevel(covers, linear_clamp, uv, lod).rgb * params.cover_light;
  let triad = fract(s.x * asp * LINES * 1.3) * 3.0;
  let mask = vec3f(f32(triad < 1.0), f32(triad >= 1.0 && triad < 2.0), f32(triad >= 2.0));
  let fine = 1.0 - smoothstep(0.15, 0.4, foot * LINES * 1.3 * 3.0);
  return c * mix(vec3f(1.0), 0.4 + 1.8 * mask, fine * 0.6);
}

fn picture(sc: Screen, s: vec2f, asp: f32, foot: f32) -> vec3f {
  let fade = smoothstep(0.5, 1.5, foot * LINES);
  if (sc.chan == 0u) {
    // Snow is grey, with a touch of the phosphor's colour.
    let v = snow(s, asp, sc.seed, sc.amt, fade);
    return mix(vec3f(v), phosphor(v), 0.3);
  }
  if (sc.chan == 1u) {
    return phosphor(scope(s, asp, sc.seed, fade));
  }
  if (sc.chan == 2u) {
    return cover(s, asp, sc.seed, sc.amt, foot);
  }
  return vec3f(0.0);
}

// The light of a screen at s (0..1 on the glass), foot a pixel's height in picture heights.
fn screen_light(sc: Screen, s0: vec2f, asp: f32, foot: f32) -> vec3f {
  // The tube's curve: the picture bows out towards its corners.
  let c = s0 - 0.5;
  let s = c * (1.0 + 0.08 * dot(c, c)) + 0.5;
  var v = picture(sc, s, asp, foot);
  // A change of channel lights the whole tube for a moment.
  v += phosphor(sc.flash * 1.1);
  // Scan lines, gone where a pixel covers more than one.
  let lines = 0.5 + 0.5 * cos(s.y * LINES * 6.2831853);
  v *= mix(1.0, 0.6 + 0.4 * lines, 1.0 - smoothstep(0.35, 0.6, foot * LINES));
  // Darker towards the edges, as a tube's picture is.
  v *= clamp(1.0 - 1.6 * dot(c, c), 0.0, 1.0);
  return max(v, vec3f(0.0)) * params.brightness;
}

// ---- Surfaces ----

// A cabinet's finish: its colour, how much it shines and how sharply, whether it is wood, and how
// old the set is (0 new, 1 worn out).
struct Finish {
  albedo: vec3f,
  shine: f32,
  sharp: f32,
  wood: bool,
  age: f32,
}

// How old a set is: most of them have seen years.
fn age_of(s: Set) -> f32 {
  return clamp(pow(hash(s.id + 0.47), 0.6) * params.wear, 0.0, 1.0);
}

// A finish as this set wears it: no two of one colour alike, and the years on it. Light plastic
// yellows, dark plastic fades towards grey-brown, and the shine goes dull.
fn aged(f: Finish, s: Set) -> Finish {
  var a = f;
  a.age = age_of(s);
  let tint = hash(s.id + 0.67) - 0.5;
  a.albedo *= (0.8 + 0.4 * hash(s.id + 0.61)) * vec3f(1.0 + 0.12 * tint, 1.0, 1.0 - 0.12 * tint);
  if (!f.wood) {
    let light = smoothstep(0.05, 0.2, rv_luma(f.albedo));
    a.albedo = mix(a.albedo, a.albedo * vec3f(1.06, 0.94, 0.66), a.age * light);
    a.albedo += vec3f(0.022, 0.019, 0.015) * a.age * (1.0 - light);
  } else {
    // Wood bleaches a little.
    a.albedo = mix(a.albedo, a.albedo * 1.4 + vec3f(0.015, 0.01, 0.005), a.age * 0.35);
  }
  a.shine *= 1.0 - 0.6 * a.age;
  a.sharp *= 1.0 - 0.4 * a.age;
  return a;
}

fn finish_of(s: Set) -> Finish {
  return aged(base_finish(s), s);
}

fn base_finish(s: Set) -> Finish {
  let h = hash(s.id + 0.53);
  // Each model in the finishes it was sold in: 0 charcoal, 1 cream, 2 silver, 3 walnut,
  // 4 gloss black, 5 red.
  var f = 0u;
  switch s.model {
    case 0u: {
      f = select(select(0u, 1u, h > 0.4), 5u, h > 0.75);
    }
    case 1u: {
      f = select(select(2u, 0u, h > 0.45), 1u, h > 0.8);
    }
    case 2u: {
      f = select(3u, 0u, h > 0.6);
    }
    case 3u: {
      f = select(select(1u, 2u, h > 0.5), 4u, h > 0.8);
    }
    case 4u: {
      f = 3u;
    }
    case 5u: {
      f = select(2u, 4u, h > 0.5);
    }
    default: {
      f = select(3u, 1u, h > 0.5);
    }
  }
  // The odd one in another colour.
  if (hash(s.id + 0.19) < params.odd) {
    f = 5u;
  }
  switch f {
    case 1u: {
      return Finish(vec3f(0.42, 0.37, 0.28), 0.3, 30.0, false, 0.0);
    }
    case 2u: {
      return Finish(vec3f(0.2, 0.21, 0.22), 0.45, 40.0, false, 0.0);
    }
    case 3u: {
      return Finish(vec3f(0.17, 0.08, 0.035), 0.35, 30.0, true, 0.0);
    }
    case 4u: {
      return Finish(vec3f(0.012), 1.2, 120.0, false, 0.0);
    }
    case 5u: {
      return Finish(vec3f(0.3, 0.045, 0.03), 0.35, 30.0, false, 0.0);
    }
    default: {
      return Finish(vec3f(params.cabinet), 0.25, 24.0, false, 0.0);
    }
  }
}

// The face's own axes, u and v across it, for a box face of normal n.
fn face_axes(n: vec3f) -> mat2x3f {
  let an = abs(n);
  if (an.y > 0.5) {
    return mat2x3f(vec3f(1.0, 0.0, 0.0), vec3f(0.0, 0.0, 1.0));
  }
  if (an.x > 0.5) {
    return mat2x3f(vec3f(0.0, 0.0, 1.0), vec3f(0.0, 1.0, 0.0));
  }
  return mat2x3f(vec3f(1.0, 0.0, 0.0), vec3f(0.0, 1.0, 0.0));
}

// The moulded plastic at p: the texture's normal bend, how open the spot is and its tone.
fn plastic(p: vec3f, axes: mat2x3f, foot: f32, seed: vec2f) -> vec4f {
  let uv = vec2f(dot(p, axes[0]), dot(p, axes[1])) / 0.4 + seed;
  let lod = log2(max(foot * 512.0 / 0.4, 1.0));
  let tex = textureSampleLevel(asset_plastic, linear_repeat, uv, lod);
  return vec4f((tex.xy * 2.0 - 1.0) * params.surface, mix(1.0, tex.z, params.surface), mix(0.5, tex.w, params.surface));
}

// Walnut veneer: the grain runs along u, wavering; its tone and a faint ridge across it.
fn wood(p: vec3f, axes: mat2x3f, foot: f32, seed: f32) -> vec2f {
  let u = dot(p, axes[0]);
  let v = dot(p, axes[1]) + 0.012 * rv_noise(vec2f(u * 3.0, seed)) + 0.004 * rv_noise(vec2f(u * 20.0, seed + 5.0));
  let ring = fract(v * 70.0 + seed);
  let fine = 1.0 - smoothstep(0.004, 0.012, foot);
  let streak = mix(0.5, smoothstep(0.0, 0.5, ring) * smoothstep(1.0, 0.6, ring), fine);
  let broad = 0.5 + 0.5 * rv_noise(vec2f(u * 1.5, v * 12.0 + seed));
  return vec2f(0.7 + 0.35 * streak + 0.25 * broad, (ring - 0.5) * 0.15 * fine);
}

// The cabinet's edges rounded off: within r of an edge the face turns towards it. Where a pixel
// is wider than the rounding it is widened and bent less, so a far edge does not flicker.
fn rounded(n: vec3f, q: vec3f, half: vec3f, axes: mat2x3f, r: f32, foot: f32) -> vec3f {
  let wide = max(r, foot * 1.5);
  let lean = r / wide;
  let eu = clamp(1.0 - (dot(half, abs(axes[0])) - abs(dot(q, axes[0]))) / wide, 0.0, 1.0) * lean;
  let ev = clamp(1.0 - (dot(half, abs(axes[1])) - abs(dot(q, axes[1]))) / wide, 0.0, 1.0) * lean;
  return n * sqrt(max(1.0 - eu * eu - ev * ev, 0.0)) + axes[0] * sign(dot(q, axes[0])) * eu + axes[1] * sign(dot(q, axes[1])) * ev;
}

// The cabinet's own surface at p on a face of normal n0: the plastic's bend or the veneer's, its
// colour, how open it is and how much of its shine is left. The years show as blotches of grime,
// dust settled on top, more scratches, and edges rubbed through.
struct Skin {
  n: vec3f,
  albedo: vec3f,
  open: f32,
  dull: f32,
}

fn skin(p: vec3f, n0: vec3f, axes: mat2x3f, s: Set, fin: Finish, foot: f32) -> Skin {
  var k: Skin;
  let at = vec2f(dot(p, axes[0]), dot(p, axes[1])) + s.id * vec2f(0.37, 0.71);
  if (fin.wood) {
    let w = wood(p, axes, foot, hash(s.id + 0.11) * 40.0);
    k.n = normalize(n0 + axes[1] * w.y);
    k.albedo = fin.albedo * w.x;
    k.open = 1.0;
  } else {
    let tex = plastic(p, axes, foot, s.id * vec2f(0.37, 0.71));
    let scratches = 0.6 + 0.9 * fin.age;
    k.n = normalize(n0 + (axes[0] * tex.x + axes[1] * tex.y) * scratches);
    k.albedo = fin.albedo * (0.7 + 0.6 * tex.w);
    k.open = tex.z;
  }
  k.dull = 1.0;
  // Grime in blotches, a hand's breadth and smaller.
  let g = rv_noise(at * 7.0) * 0.6 + rv_noise(at * 26.0) * 0.4;
  k.albedo *= 1.0 - 0.6 * fin.age * smoothstep(-0.15, 0.5, g);
  // Dust on what faces up.
  if (n0.y > 0.5) {
    let dust = fin.age * (0.55 + 0.45 * smoothstep(-0.4, 0.5, rv_noise(at * 4.0)));
    k.albedo = mix(k.albedo, vec3f(0.17, 0.16, 0.14), dust * 0.85);
    k.dull = 1.0 - dust;
  }
  // Edges rubbed: plastic scuffed paler, veneer through to the raw wood.
  let q = p - s.centre;
  let edge = min(dot(s.half, abs(axes[0])) - abs(dot(q, axes[0])), dot(s.half, abs(axes[1])) - abs(dot(q, axes[1])));
  let rub = fin.age * (1.0 - smoothstep(0.0, 0.018 + 0.012 * rv_noise(at * 40.0), edge)) * smoothstep(-0.2, 0.3, rv_noise(at * 9.0 + 3.0));
  k.albedo = mix(k.albedo, select(k.albedo * 1.6 + vec3f(0.03), vec3f(0.3, 0.21, 0.13), fin.wood), rub);
  return k;
}

// How much of the key light reaches p: the slots walked once more, towards the light.
fn keylit(p: vec3f, n: vec3f) -> f32 {
  if (dot(n, KEY_DIR) <= 0.0) {
    return 0.0;
  }
  if (params.shadows <= 0.0) {
    return 1.0;
  }
  let h = trace(p + n * 0.003, KEY_DIR, 10);
  return 1.0 - params.shadows * f32(h.kind == 1u);
}

// The light of the screen of the set beside s (on the side of normal n) on a side face at p: the
// screen as a glowing rectangle, seen at an angle.
fn spill(p: vec3f, n: vec3f, s: Set) -> vec3f {
  let o = set_at_point(s.centre.x + sign(n.x) * (s.half.x + 0.08), p.y);
  if (!o.on) {
    return vec3f(0.0);
  }
  let m = model_of(o.model);
  let low = o.centre - o.half + vec3f(0.0, o.plinth, 0.0);
  let mid = vec3f(low.x + 0.5 * (m.screen.x + m.screen.z), low.y + 0.5 * (m.screen.y + m.screen.w), low.z);
  let l = mid - p;
  let r2 = dot(l, l);
  let r = sqrt(r2);
  let cos_in = max(dot(n, l / r), 0.0);
  let cos_out = max(l.z / r, 0.0);
  let area = (m.screen.z - m.screen.x) * (m.screen.w - m.screen.y);
  return screen_glow(screen_of(o)) * area * cos_in * cos_out / (3.14159 * r2 + 0.02) * params.spill;
}

// The key light falls in pools along the wall, as from lamps overhead, dark between them: a pool
// every few metres, each its own strength.
fn pool(p: vec3f) -> f32 {
  let size = vec2f(3.2, 2.8);
  let base = floor(p.xy / size);
  var sum = 0.0;
  for (var i = -1.0; i <= 1.0; i += 1.0) {
    for (var j = -1.0; j <= 1.0; j += 1.0) {
      let cell = base + vec2f(i, j);
      let at = (cell + 0.5 + (vec2f(hash(cell + 0.17), hash(cell + 0.59)) - 0.5) * 0.6) * size;
      let r = length(p.xy - at);
      sum += (0.6 + 0.6 * hash(cell + 0.83)) * exp(-r * r / 1.8);
    }
  }
  return mix(1.0, 0.45 + 0.7 * min(sum, 1.2), params.pools);
}

// Light on a surface: the key light (and its shine), the room's faint fill, and what is given.
fn lit(albedo: vec3f, n: vec3f, d: vec3f, key: f32, open: f32, shine: f32, sharp: f32, extra: vec3f) -> vec3f {
  let diffuse = max(dot(n, KEY_DIR), 0.0) * key;
  let h = normalize(KEY_DIR - d);
  let spec = pow(max(dot(n, h), 0.0), sharp) * (sharp + 8.0) / 25.0 * shine * key * f32(dot(n, KEY_DIR) > 0.0);
  let fill = vec3f(0.6, 0.65, 0.75) * params.fill * (0.4 + 0.6 * max(n.y, 0.0)) * open;
  return albedo * (state.key * diffuse + fill + extra) + state.key * spec;
}

// The rounded rectangle of half size b and corner r: distance (negative inside).
fn rrect(p: vec2f, b: vec2f, r: f32) -> f32 {
  let q = abs(p) - b + r;
  return length(max(q, vec2f(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

// The front of a set: screen and its sunken surround, knobs, grille, the strip of metal or cloth,
// and the cabinet around them. u is from the front's lower left corner.
fn shade_front(p: vec3f, d: vec3f, s: Set, foot: f32, key: f32) -> vec3f {
  let m = model_of(s.model);
  let fin = finish_of(s);
  let low = s.centre - s.half;
  // From the model's lower left corner: a taller foot lifts the whole front.
  let u = p.xy - low.xy - vec2f(0.0, s.plinth);
  let axes = face_axes(vec3f(0.0, 0.0, -1.0));
  let q = p - s.centre;
  var n = vec3f(0.0, 0.0, -1.0);
  let sc = screen_of(s);
  let glow = screen_glow(sc);

  // The screen.
  let sh = 0.5 * (m.screen.zw - m.screen.xy);
  let sp = u - 0.5 * (m.screen.xy + m.screen.zw);
  let corner = m.style.y * min(sh.x, sh.y);
  let sd = rrect(sp, sh, corner);
  if (sd < 0.0) {
    let g = sp / sh;
    // The glass bows out: its normal leans away from the middle.
    let gn = normalize(vec3f(g * 0.2, -1.0));
    var c = screen_light(sc, sp / (2.0 * sh) + 0.5, sh.x / sh.y, foot / (2.0 * sh.y));
    // The picture stops short of the glass's edge, in the shadow of the surround.
    c *= mix(0.15, 1.0, smoothstep(0.0, 0.025 * min(sh.x, sh.y) / 0.15, -sd));
    // The dark glass itself, and what it reflects: the dim room and the key light.
    let cosv = max(dot(gn, -d), 0.0);
    let fres = 0.04 + 0.96 * pow(1.0 - cosv, 5.0);
    let r = reflect(d, gn);
    let room = vec3f(0.006, 0.007, 0.009) * (0.5 + 0.5 * r.y);
    let toward = max(dot(r, KEY_DIR), 0.0);
    let glare = state.key * pow(toward, 900.0) * 6.0 * key;
    c += vec3f(0.018, 0.021, 0.02) * (state.key * max(dot(gn, KEY_DIR), 0.0) * key + vec3f(params.fill));
    c += (room + glare) * mix(fres, 1.0, 0.08) * params.glass;
    // The broad sheen of the light across the curved glass.
    c += state.key * pow(toward, 6.0) * 0.25 * (0.25 + fres) * key * params.glass;
    // Years of dust and fingers on the glass: a pale film, thicker in places, catching the light.
    let film = fin.age * (0.4 + 0.6 * smoothstep(-0.2, 0.6, rv_noise(sp * 18.0 + s.id)));
    c = c * (1.0 - 0.12 * film) + film * 0.02 * (state.key * key + vec3f(params.fill * 4.0));
    // A price sticker on a corner of the glass, or a strip of tape across one.
    let tag = hash(s.id + 0.29);
    if (tag < params.wear * 0.45) {
      let cx = select(-1.0, 1.0, hash(s.id + 0.31) < 0.5);
      let cy = select(-1.0, 1.0, hash(s.id + 0.37) < 0.5);
      let at = sh - vec2f(0.05, 0.04) - corner * 0.3;
      let local = (sp - vec2f(cx, cy) * at) * vec2f(cx, cy);
      if (tag < params.wear * 0.28) {
        let size = vec2f(0.03, 0.019);
        let r = rrect(local, size, 0.002);
        if (r < 0.0) {
          // Paper, yellowed, with lines of print too small to read.
          let lines = step(0.5, fract((local.y + size.y) / 0.006)) * step(abs(local.x), size.x * 0.75) * step(abs(local.y), size.y * 0.7);
          let fine = 1.0 - smoothstep(0.001, 0.003, foot);
          let paper = mix(vec3f(0.62, 0.6, 0.52), vec3f(0.6, 0.5, 0.3), fin.age) * (1.0 - 0.6 * lines * fine);
          return lit(paper, gn, d, key, 1.0, 0.1, 12.0, glow * 0.3);
        }
      } else {
        // Tape across the corner, its ends torn.
        let along = (local.x + local.y) * 0.7071;
        let across = (local.x - local.y) * 0.7071;
        if (abs(across) < 0.011 && along > -0.06 + 0.004 * rv_noise(vec2f(across * 300.0, 1.0)) && along < 0.05) {
          let tape = vec3f(0.42, 0.37, 0.24) * (1.0 + 0.3 * fin.age);
          return mix(c, lit(tape, gn, d, key, 1.0, 0.3, 20.0, glow * 0.4), 0.85);
        }
      }
    }
    return c;
  }

  let k0 = skin(p, n, axes, s, fin, foot);
  var albedo = k0.albedo;
  var open = k0.open;
  var bent = k0.n - n;
  var shine = fin.shine * k0.dull;
  var sharp = fin.sharp;
  var plain = true;

  // The surround sinks into the screen: within `ring` of it the face slopes in, lit by it.
  let ring = 0.016;
  var extra = glow * 0.7 * exp(-sd / 0.02);
  if (sd < ring) {
    let e = 0.002;
    let grad = normalize(vec2f(rrect(sp + vec2f(e, 0.0), sh, corner) - rrect(sp - vec2f(e, 0.0), sh, corner),
      rrect(sp + vec2f(0.0, e), sh, corner) - rrect(sp - vec2f(0.0, e), sh, corner)));
    let lean = 0.9 * clamp(ring / max(foot * 1.5, ring), 0.0, 1.0);
    n = normalize(vec3f(-grad * lean, -1.0));
    open *= 0.75;
    extra = glow * 1.6 * (1.0 - sd / ring);
    // Light cabinets have a dark surround.
    albedo = min(albedo, vec3f(0.03));
  }

  // The strip: brushed metal along its length, or speaker cloth.
  let ph = 0.5 * (m.panel.zw - m.panel.xy);
  let pp = u - 0.5 * (m.panel.xy + m.panel.zw);
  let along_x = ph.x > ph.y;
  let pd = rrect(pp, ph, 0.004);
  if (pd < 0.0 && sd >= ring) {
    plain = false;
    if (m.style.x > 0.5) {
      // Cloth: a fine weave, matt, warm.
      let fine = 1.0 - smoothstep(0.0008, 0.003, foot);
      let wv = abs(fract(u / 0.0025) - 0.5) + abs(fract(u.yx / 0.0025) - 0.5);
      let weave = mix(0.5, 0.5 + 0.5 * sin((wv.x - wv.y) * 6.28), fine);
      albedo = select(vec3f(0.11, 0.085, 0.06), vec3f(0.2, 0.18, 0.14), !fin.wood) * (0.8 + 0.3 * weave);
      bent = vec3f(0.0);
      n = normalize(vec3f((weave - 0.5) * 0.3 * fine, 0.0, -1.0));
      // Sunk a little behind its frame.
      open *= 0.8 * mix(0.6, 1.0, smoothstep(0.0, 0.008, -pd));
      shine = 0.03;
      sharp = 8.0;
    } else {
      let across = select(u.x, u.y, along_x);
      // Brush marks: fine streaks across the strip's width, averaged away when far.
      let streak = hash(vec2f(floor(across * 2400.0), s.id.x)) - 0.5;
      let fine = 1.0 - smoothstep(0.0003, 0.0012, foot);
      let b = streak * 0.25 * fine;
      bent = vec3f(0.0);
      n = normalize(vec3f(select(b, 0.0, along_x), select(0.0, b, along_x), -1.0));
      let edge = clamp(1.0 + pd / max(0.003, foot * 1.5), 0.0, 1.0);
      n = normalize(n + vec3f(normalize(pp) * edge * 0.5, 0.0) * f32(pd > -0.003));
      albedo = vec3f(0.3, 0.31, 0.32) * (0.92 + 0.16 * streak * fine);
      // Old metal goes dull and spotted.
      albedo *= 1.0 - 0.45 * fin.age * smoothstep(0.0, 0.5, rv_noise(u * 40.0 + s.id));
      shine = 1.4 * (1.0 - 0.6 * fin.age);
      sharp = 60.0;
      open = 1.0;
    }
    extra *= 0.4;
  }

  // The grille: a field of small holes, merging into a darker grey when far.
  let gh = 0.5 * (m.grille.zw - m.grille.xy);
  let gp = u - 0.5 * (m.grille.xy + m.grille.zw);
  if (rrect(gp, gh, 0.003) < 0.0) {
    let pitch = 0.0075;
    let cell = vec2f(pitch, pitch * 0.866);
    let row = floor(gp.y / cell.y);
    let gx = gp.x + select(0.0, 0.5 * pitch, fract(row * 0.5) > 0.25);
    let local = vec2f(fract(gx / pitch) - 0.5, fract(gp.y / cell.y) - 0.5) * cell;
    let hole = length(local) - 0.0024;
    let near = 1.0 - smoothstep(pitch * 0.25, pitch * 0.6, foot);
    let inside = smoothstep(foot * 0.5, -foot * 0.5, hole) * near;
    let rim = smoothstep(0.0012, 0.0, abs(hole)) * near;
    if (hole < 0.0012) {
      n = normalize(n + vec3f(-normalize(local + 1e-5) * rim * 0.6, 0.0));
    }
    // The holes are black; from afar the field is just darker.
    open *= (1.0 - 0.45 * (1.0 - near)) * (1.0 - 0.92 * inside);
    albedo *= mix(0.55, 1.0, near);
  }

  // The knobs: raised discs, the rim knurled, a line pointing. Dark on plastic, cream on wood.
  for (var k = 0; k < 2; k++) {
    let kb = select(m.knob_b, m.knob_a, k == 0);
    let kp = u - kb.xy;
    let kr = length(kp) / kb.z;
    if (kr < 1.0 && hash(s.id + f32(k) * 5.9) < params.wear * 0.08) {
      // A knob pulled off: the hole and its shaft.
      bent = vec3f(0.0);
      plain = false;
      albedo = select(vec3f(0.005), vec3f(0.25), kr < 0.18);
      shine = select(0.0, 0.8, kr < 0.18);
      n = vec3f(0.0, 0.0, -1.0);
      open = select(0.3, 1.0, kr < 0.18);
      extra = vec3f(0.0);
    } else if (kr < 1.0) {
      let out = kp / max(length(kp), 1e-5);
      let edge = smoothstep(0.78, 1.0, kr);
      let a = atan2(kp.y, kp.x);
      let knurl = sin(a * 36.0) * smoothstep(0.84, 0.9, kr) * (1.0 - smoothstep(0.0005, 0.002, foot));
      let tangent = vec2f(-out.y, out.x);
      n = normalize(vec3f(out * edge * 1.6 + tangent * knurl * 0.3, -1.0));
      bent = vec3f(0.0);
      albedo = select(vec3f(0.03), vec3f(0.4, 0.37, 0.3), fin.wood);
      shine = 0.9;
      sharp = 40.0;
      open = 1.0;
      plain = false;
      // The knobs stand out of the face, into the screen's light.
      extra = glow * 0.6;
      let turn = hash(s.id + f32(k) * 3.3) * 5.0 - 2.5;
      let pointer = vec2f(sin(turn), cos(turn));
      let along = dot(kp, pointer);
      let side = abs(dot(kp, vec2f(-pointer.y, pointer.x)));
      if (along > kb.z * 0.25 && along < kb.z * 0.8 && side < kb.z * 0.06 + foot * 0.5) {
        albedo = select(vec3f(0.6), vec3f(0.03), fin.wood);
      }
    }
  }

  if (plain) {
    n = normalize(n + bent);
  }
  // Darker towards the foot, where the set sits on the one below.
  open *= mix(0.45, 1.0, smoothstep(0.0, 0.3, (p.y - low.y) / (2.0 * s.half.y)));
  n = normalize(rounded(n, q, s.half, axes, 0.012, foot));
  return lit(albedo, n, d, key, open, shine, sharp, extra) * open;
}

// The sides, top and bottom of a cabinet, with the seam where the front meets the shell.
fn shade_side(p: vec3f, d: vec3f, n0: vec3f, s: Set, foot: f32, key: f32) -> vec3f {
  let axes = face_axes(n0);
  let q = p - s.centre;
  let fin = finish_of(s);
  let k = skin(p, n0, axes, s, fin, foot);
  var open = k.open;
  // The seam, a few centimetres behind the front.
  let behind = p.z - (s.centre.z - s.half.z);
  let seam = 1.0 - smoothstep(0.0, max(0.0015, foot), abs(behind - 0.035));
  open *= 1.0 - 0.7 * seam * f32(!fin.wood);
  let n = normalize(rounded(k.n, q, s.half, axes, 0.012, foot));
  // In the gap between sets the faces see little of the room.
  let deep = clamp(-p.z / (2.0 * s.half.z), 0.0, 1.0);
  open *= mix(0.35, 1.0, 1.0 - deep);
  var extra = vec3f(0.0);
  if (abs(n0.x) > 0.5) {
    extra = spill(p, n0, s);
  }
  return lit(k.albedo, n, d, key, open, fin.shine * k.dull, fin.sharp, extra) * open;
}

fn shade(px: Pixel) -> vec4f {
  let ray = camera_ray(px);
  let h = trace(ray.o, ray.d, 24);
  if (h.kind == 0u) {
    return vec4f(0.0, 0.0, 0.0, 1.0);
  }
  let p = ray.o + ray.d * h.t;
  // A pixel's width where the ray met, in metres, across the face.
  let pixel = 2.0 * state.lens / px.size.y;
  let foot = h.t * pixel / max(abs(dot(h.n, ray.d)), 0.2);
  let key = keylit(p, h.n) * pool(p);
  var c: vec3f;
  if (h.kind == 2u) {
    // The wall behind: matt black paint, and the shadows the sets throw on it.
    c = lit(vec3f(0.025), h.n, ray.d, key, 0.5, 0.1, 8.0, vec3f(0.0)) * 0.5;
  } else if (h.n.z < -0.5) {
    c = shade_front(p, ray.d, h.tv, foot, key);
  } else {
    c = shade_side(p, ray.d, h.n, h.tv, foot, key);
  }
  return vec4f(c, 1.0);
}
