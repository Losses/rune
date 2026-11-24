// The tunnel as the camera sees it: the core's light far ahead and its rays, the speed lines
// rushing past on the tunnel's unseen walls, haze; and in front of them, the things of this
// pixel's tile, the nearest one met, the sparkles and the small lights in front of it laid over.
// Everything but the emoji is then graded into the look the runtime has chosen (see `grade`).

// The tunnel: the core's glow along +z, rays round it, the lines on three walls further and
// further out, and the haze.
fn backdrop(o: vec3f, d: vec3f, pixel: f32) -> vec3f {
  let ahead = max(d.z, 0.0);
  var c = mix(state.colours[0].rgb, state.colours[1].rgb, smoothstep(0.0, 1.0, d.z));
  let core = params.core * mix(0.75, 1.25, input.energy);
  // As big on the screen through a longer lens (which would spread its glow over the picture).
  let zoom = pow(state.focal / params.focal, 2.0);
  c += state.colours[2].rgb * core * (pow(ahead, 1500.0 * zoom) * 8.0 + pow(ahead, 150.0 * zoom) * 1.0 + pow(ahead, 14.0 * zoom) * 0.15);
  c += state.colours[3].rgb * core * pow(ahead, 8.0 * zoom) * 0.04;
  // Rays from the core: brighter and dimmer by the way round it, slowly turning (on the scene's
  // time, so they stand still in silence).
  let round = normalize(d.xy + vec2f(1e-6, 0.0));
  let rays = pow(0.5 + 0.5 * rv_noise(round * 5.0 + vec2f(state.phase * 0.05, 0.0)), 6.0);
  c += state.colours[4].rgb * core * rays * pow(ahead, 30.0 * zoom) * 1.5;

  let a = dot(d.xy, d.xy);
  if (a < 1e-8) {
    return c;
  }
  let b = dot(o.xy, d.xy);
  // The speed lines: on three walls, a few lines each at its own angle, in dashes of their own
  // lengths that rush past as the camera flies.
  for (var w = 0.0; w < 3.0; w += 1.0) {
    let radius = 4.5 * pow(1.8, w);
    let lines = 40.0 + 40.0 * w;
    let t = (-b + sqrt(b * b - a * (dot(o.xy, o.xy) - radius * radius))) / a;
    let p = o + d * t;
    let cell = (atan2(p.y, p.x) / 6.2831853 + 0.5) * lines;
    let idx = floor(cell);
    let key = vec2f(idx, w * 7.0);
    if (hash(key + 0.1) < params.lines) {
      let shift = (hash(key + 0.2) - 0.5) * 0.6;
      let across = abs(fract(cell) - 0.5 - shift) * 6.2831853 * radius / lines;
      // Never thinner than a pixel; dimmer instead.
      let foot = t * pixel;
      let width = 0.012;
      let wide = max(width, foot);
      let cover = clamp((wide * 0.5 - across) / foot + 0.5, 0.0, 1.0) * width / wide;
      let len = mix(3.0, 25.0, hash(key + 0.3));
      let seg = fract((p.z + hash(key + 0.4) * len) / len);
      let dash = smoothstep(0.0, 0.08, seg) * (1.0 - smoothstep(0.55, 0.65, seg));
      let fade = exp(-t * 0.015) * smoothstep(1.0, 5.0, t);
      c += state.colours[5].rgb * cover * dash * fade * 0.5;
    }
  }
  // Glints: on two walls near the middle, a point here and there in each cell of angle and
  // length, of four colours; far off, where a pixel covers many, they become the glittering burst
  // round the core.
  for (var w = 0.0; w < 2.0; w += 1.0) {
    let radius = 2.4 + 2.6 * w;
    let t = (-b + sqrt(b * b - a * (dot(o.xy, o.xy) - radius * radius))) / a;
    let p = o + d * t;
    let around = (atan2(p.y, p.x) / 6.2831853 + 0.5) * 70.0;
    let along = p.z / 1.3;
    let cell = floor(vec2f(around, along));
    let key = cell + vec2f(w * 131.0, 0.0);
    if (hash(key + 0.5) < params.glints) {
      let at = cell + 0.5 + (vec2f(hash(key + 0.6), hash(key + 0.7)) - 0.5) * 0.6;
      let off = (vec2f(around, along) - at) * vec2f(6.2831853 * radius / 70.0, 1.3);
      let foot = t * pixel;
      let size = 0.035;
      let wide = max(size, foot);
      let glow = smoothstep(wide, 0.0, length(off)) * (size / wide) * (size / wide);
      let hc = hash(key + 0.8);
      let tint = state.colours[select(select(select(6u, 7u, hc > 0.4), 8u, hc > 0.7), 9u, hc > 0.85)].rgb;
      c += tint * glow * 1.8 * smoothstep(12.0, 40.0, t);
    }
  }

  // Violet haze drifting on the far wall.
  let radius = 13.0;
  let t = (-b + sqrt(b * b - a * (dot(o.xy, o.xy) - radius * radius))) / a;
  let p = o + d * t;
  let n = rv_fbm(vec2f(p.x * 0.07 + p.z * 0.015, p.y * 0.07 - p.z * 0.01), 3);
  c += state.colours[10].rgb * smoothstep(0.1, 0.5, n) * 0.22 * params.clouds * exp(-t * 0.012);
  return c;
}

// A sparkle: four thin rays and a bright point, on the screen, twinkling on the scene's time.
fn sparkle(i: u32, at: vec2f) -> vec3f {
  let sc = state.scr[i];
  let q = (at - sc.xy) / sc.z;
  let rays = exp(-abs(q.y) * 30.0) * max(1.0 - abs(q.x), 0.0) + exp(-abs(q.x) * 30.0) * max(1.0 - abs(q.y), 0.0);
  let core = exp(-dot(q, q) * 40.0);
  let twinkle = 0.6 + 0.4 * sin(state.phase * mix(3.0, 7.0, hash(vec2f(f32(i), 3.3))) + f32(i));
  let fade = exp(-sc.w * 0.02);
  return state.colours[11].rgb * (rays * 1.2 + core * 3.0) * twinkle * fade * 0.9;
}

// How bright a light's core is (HDR, against what the lens flare takes for a highlight: above 1
// and 1.5 times what is round it, after a 13-tap average at a quarter of the size), and how much of
// that its glow is, at the glow's heart. The glow is soft and dim, so that the tunnel is hardly
// brighter for the lights.
const LIGHT_CORE: f32 = 20.0;
const LIGHT_GLOW: f32 = 0.35;

// A light: a small bright disc with a soft glow round it, on the screen, steady (it does not
// flash, twinkle or follow the beat), coming out of the tunnel's light far off like the other
// things, and fading out as it comes near, before it is no longer drawn (3 m: see `project`), so
// that none goes out at once as it passes. `scr.z` is the glow's radius on the screen, the core a
// fifth of it; a light further off than its core is a pixel across is dimmed by its area, so it
// never swells to a larger point than it is.
fn light(i: u32, at: vec2f) -> vec3f {
  let sc = state.scr[i];
  let d = length(at - sc.xy) / sc.z;
  let true_glow = state.focal * state.pos[i].w / sc.w;
  let area = min(1.0, true_glow * true_glow / (sc.z * sc.z));
  let core = 1.0 - smoothstep(0.14, 0.22, d);
  let glow = exp(-d * d * 9.0);
  let fade = (1.0 - smoothstep(55.0, 95.0, sc.w)) * smoothstep(3.0, 7.0, sc.w);
  return state.colours[11].rgb * (core * LIGHT_CORE + glow * LIGHT_GLOW) * state.ry[i].w * area * fade;
}

// The mip of the emoji atlas for an emoji of size `size` at depth z: how many of its 256 pixels
// one pixel of the screen covers.
fn emoji_lod(z: f32, pixel: f32, size: f32) -> f32 {
  return log2(max(z * pixel / (2.0 * size) * 256.0, 1.0));
}

// ---- The grade ----
//
// The picture, but for the emoji, mapped by its lightness onto one colour, as Neon Street's and CRT
// Wall's are (the picture's own light is kept: what glows still glows and the dark stays dark):
//   - the grey look: greys with a faint tint of the theme colour, a little dimmer than the picture
//     was;
//   - the theme colour look: a ramp from a dark shade through the theme colour to near white
//     (state.theme_ramp).
// `state.colour` fades from one to the other. The emoji keep their own colours: their light is
// carried apart and added after the grade.

// A colour with the lightness l.
fn at_lightness(c: vec3f, l: f32) -> vec3f {
  return c / max(rv_luma(c), 1e-4) * l;
}

fn ramp(t: f32) -> vec3f {
  let x = clamp(t, 0.0, 1.0) * 3.0;
  let r = state.theme_ramp;
  if (x < 1.0) {
    return mix(r[0].rgb, r[1].rgb, x);
  }
  if (x < 2.0) {
    return mix(r[1].rgb, r[2].rgb, x - 1.0);
  }
  return mix(r[2].rgb, r[3].rgb, x - 2.0);
}

// The lightness at which the theme colour's ramp is half way up.
const TONE_KEY: f32 = 0.2;
// How dark the grey look is against the picture, and how much of the theme colour tints its greys.
const GREY_DIM: f32 = 0.85;
const GREY_TINT: f32 = 0.07;

fn grade(c: vec3f) -> vec3f {
  let l = rv_luma(c);
  var out = vec3f(0.0);
  if (state.colour < 0.999) {
    let tint = state.theme_ramp[2].rgb / max(rv_luma(state.theme_ramp[2].rgb), 1e-3);
    out = at_lightness(mix(vec3f(1.0), tint, GREY_TINT), l * GREY_DIM) * (1.0 - state.colour);
  }
  if (state.colour > 0.001) {
    out += at_lightness(ramp(l / (l + TONE_KEY)), l) * state.colour;
  }
  return out;
}

fn shade(px: Pixel) -> vec4f {
  let o = state.cam_pos;
  let d = normalize(state.cam_fwd * state.focal + state.cam_right * px.p.x + state.cam_up * px.p.y);
  // How wide one pixel is, as an angle.
  let pixel = 1.0 / (state.focal * px.size.y);
  let bg = backdrop(o, d, pixel);

  let tx = min(u32(px.uv.x * f32(TX)), TX - 1u);
  let ty = min(u32(px.uv.y * f32(TY)), TY - 1u);
  let k = ty * TX + tx;
  let count = state.tile_n[k];
  var best = vec4f(1e9, 0.0, 0.0, 0.0);
  var who = 0u;
  for (var j = 0u; j < count; j++) {
    let i = state.tile[k * LIST + j];
    let sc = state.scr[i];
    let off = px.p - sc.xy;
    if (dot(off, off) > sc.z * sc.z) {
      continue;
    }
    let rx = state.rx[i];
    let kind = floor(rx.w / 10.0);
    if (kind == SPARK || kind == LIGHT) {
      continue;
    }
    let pos = state.pos[i];
    let ry = state.ry[i];
    let rz = state.rz[i];
    let rel = o - pos.xyz;
    let ro = vec3f(dot(rel, rx.xyz), dot(rel, ry.xyz), dot(rel, rz.xyz)) / pos.w;
    let rd = vec3f(dot(d, rx.xyz), dot(d, ry.xyz), dot(d, rz.xyz)) / pos.w;
    var h: vec4f;
    if (kind == BADGE) {
      h = badge_hit(ro, rd, rx.w - BADGE * 10.0);
    } else if (kind == EMOJI) {
      h = emoji_hit(ro, rd, rz.w, emoji_lod(sc.w, pixel, pos.w));
    } else {
      h = flake_hit(ro, rd);
    }
    if (h.x < best.x) {
      best = h;
      who = i;
    }
  }

  // The picture in two parts: the emoji's own light (never graded) and all the rest (graded).
  var rest = bg;
  var emoji = vec3f(0.0);
  if (best.x < 1e8) {
    let i = who;
    let kind = floor(state.rx[i].w / 10.0);
    var c: vec3f;
    let pos = state.pos[i];
    let rel = o + d * best.x - pos.xyz;
    let q = vec3f(dot(rel, state.rx[i].xyz), dot(rel, state.ry[i].xyz), dot(rel, state.rz[i].xyz)) / pos.w;
    // Far off, they come out of the tunnel's light.
    let far = smoothstep(55.0, 95.0, best.x);
    if (kind == EMOJI) {
      // The pixel is the backdrop, mixed with the emoji by how much of it the emoji covers and
      // how near it is: the backdrop's part is graded, the emoji's is not.
      let e = shade_emoji(i, d, best.yzw, q, emoji_lod(state.scr[i].w, pixel, pos.w));
      let k = e.a * (1.0 - far);
      emoji = e.rgb * k;
      rest = bg * (1.0 - k);
    } else {
      if (kind == BADGE) {
        c = shade_badge(i, d, best.x, best.yzw, q, pixel);
      } else {
        c = shade_flake(i, d, best.yzw);
      }
      rest = mix(c, bg, far);
    }
  }

  // The sparkles and the lights in front of whatever was met.
  let depth = best.x * dot(d, state.cam_fwd);
  for (var j = 0u; j < count; j++) {
    let i = state.tile[k * LIST + j];
    let what = floor(state.rx[i].w / 10.0);
    if ((what == SPARK || what == LIGHT) && state.scr[i].w < depth) {
      let sc = state.scr[i];
      let off = px.p - sc.xy;
      if (dot(off, off) < sc.z * sc.z) {
        if (what == LIGHT) {
          rest += light(i, px.p);
        } else {
          rest += sparkle(i, px.p);
        }
      }
    }
  }
  return vec4f(grade(rest) + emoji, 1.0);
}
