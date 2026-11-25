#version 460
#include <flutter/runtime_effect.glsl>

// The cover wall's colour field: the slowly moving colours lib/shaders/
// gradient.frag tints the covers with. It changes over hundreds of pixels, so
// the app works it out at a fraction of the resolution (CoverWallGradient) and
// gradient.frag scales it back up.

out vec4 fragColor;

uniform vec2 resolution;

uniform float u_time;
uniform vec2 u_mouse;

uniform vec4 u_params;
uniform vec4 u_params2;

uniform vec3 u_color;
uniform vec3 u_color2;

uniform float u_is_dark;

// The cosine and sine of the two hue shifts, which are the same for every
// pixel: the slow sway (sin(u_time) * MPI / 30) and the final shift by
// u_params.z degrees.
uniform vec2 u_hue_sway;
uniform vec2 u_hue_turn;

// The gradients of the Perlin noise at the corners of the lattice cells the
// window covers. The noise is read at (scale_uv, u_time); scale_uv stays
// within (-1, 1) while u_params2.y (the scale) is below 2, so x and y lie in
// the cells from -1 to 1, and the time is the same for every pixel. The app
// computes the gradients of the 3 x 3 corners (x and y from -1 to 1) at
// floor(u_time) (u_lattice0_*) and the next integer (u_lattice1_*), normalized,
// with the hash this shader used to compute for every pixel. The names give
// x then y: m is -1, o is 0, p is 1.
uniform vec3 u_lattice0_mm;
uniform vec3 u_lattice0_om;
uniform vec3 u_lattice0_pm;
uniform vec3 u_lattice0_mo;
uniform vec3 u_lattice0_oo;
uniform vec3 u_lattice0_po;
uniform vec3 u_lattice0_mp;
uniform vec3 u_lattice0_op;
uniform vec3 u_lattice0_pp;
uniform vec3 u_lattice1_mm;
uniform vec3 u_lattice1_om;
uniform vec3 u_lattice1_pm;
uniform vec3 u_lattice1_mo;
uniform vec3 u_lattice1_oo;
uniform vec3 u_lattice1_po;
uniform vec3 u_lattice1_mp;
uniform vec3 u_lattice1_op;
uniform vec3 u_lattice1_pp;

const float MPI = 6.28318530718;

// cos mix
vec3 palette(in float t, in vec3 a, in vec3 b, in vec3 c, in vec3 d, in float e) {
    return a + b * cos(6.28318 * (c * t + d)) * e;
}

// hue shift: turns the chroma of the color in YIQ by the angle whose cosine
// and sine are cs. This is the hue + angle of the polar form, without its
// atan and sqrt.
vec3 hueShift(vec3 color, vec2 cs) {
    const vec3 kRGBToYPrime = vec3(0.299, 0.587, 0.114);
    const vec3 kRGBToI = vec3(0.596, -0.275, -0.321);
    const vec3 kRGBToQ = vec3(0.212, -0.523, 0.311);

    const vec3 kYIQToR = vec3(1.0, 0.956, 0.621);
    const vec3 kYIQToG = vec3(1.0, -0.272, -0.647);
    const vec3 kYIQToB = vec3(1.0, -1.107, 1.704);

    float YPrime = dot(color, kRGBToYPrime);
    float I0 = dot(color, kRGBToI);
    float Q0 = dot(color, kRGBToQ);

    float I = I0 * cs.x - Q0 * cs.y;
    float Q = I0 * cs.y + Q0 * cs.x;

    vec3 yIQ = vec3(YPrime, I, Q);

    return vec3(dot(yIQ, kYIQToR), dot(yIQ, kYIQToG), dot(yIQ, kYIQToB));
}

// noise
vec3 fade(vec3 t) { return t * t * t * (t * (t * 6.0 - 15.0) + 10.0); }

// The gradient at corner (x, y) of the cell whose lower corner is at
// (cell.x - 1, cell.y - 1), cell being 0 or 1 on each side: the corner one of
// a, b, c and d (x and y from the cell's lower corner, then up).
vec3 corner(vec2 cell, vec3 a, vec3 b, vec3 c, vec3 d) {
    return mix(mix(a, b, cell.x), mix(c, d, cell.x), cell.y);
}

// Classic Perlin noise at (P, t), with the corner gradients from the lattice
// uniforms.
float cnoise(vec2 P, float t) {
    vec3 Pf0 = vec3(fract(P), fract(t));
    vec3 Pf1 = Pf0 - vec3(1.0);
    vec2 cell = floor(P) + vec2(1.0);

    vec3 g000 = corner(cell, u_lattice0_mm, u_lattice0_om, u_lattice0_mo, u_lattice0_oo);
    vec3 g100 = corner(cell, u_lattice0_om, u_lattice0_pm, u_lattice0_oo, u_lattice0_po);
    vec3 g010 = corner(cell, u_lattice0_mo, u_lattice0_oo, u_lattice0_mp, u_lattice0_op);
    vec3 g110 = corner(cell, u_lattice0_oo, u_lattice0_po, u_lattice0_op, u_lattice0_pp);
    vec3 g001 = corner(cell, u_lattice1_mm, u_lattice1_om, u_lattice1_mo, u_lattice1_oo);
    vec3 g101 = corner(cell, u_lattice1_om, u_lattice1_pm, u_lattice1_oo, u_lattice1_po);
    vec3 g011 = corner(cell, u_lattice1_mo, u_lattice1_oo, u_lattice1_mp, u_lattice1_op);
    vec3 g111 = corner(cell, u_lattice1_oo, u_lattice1_po, u_lattice1_op, u_lattice1_pp);

    float n000 = dot(g000, Pf0);
    float n100 = dot(g100, vec3(Pf1.x, Pf0.yz));
    float n010 = dot(g010, vec3(Pf0.x, Pf1.y, Pf0.z));
    float n110 = dot(g110, vec3(Pf1.xy, Pf0.z));
    float n001 = dot(g001, vec3(Pf0.xy, Pf1.z));
    float n101 = dot(g101, vec3(Pf1.x, Pf0.y, Pf1.z));
    float n011 = dot(g011, vec3(Pf0.x, Pf1.yz));
    float n111 = dot(g111, Pf1);

    vec3 fade_xyz = fade(Pf0);
    vec4 n_z = mix(vec4(n000, n100, n010, n110), vec4(n001, n101, n011, n111), fade_xyz.z);
    vec2 n_yz = mix(n_z.xy, n_z.zw, fade_xyz.y);
    float n_xyz = mix(n_yz.x, n_yz.y, fade_xyz.x);
    return 2.2 * n_xyz;
}

// Animation parameters
const vec3 amplitude = vec3(0.3, 0.3, 0.3);
const vec3 frequency = vec3(0.5, 0.5, 0.5);
const vec3 phase_shift = vec3(0.2, 0.2, 0.2);

void main() {
    // params
    float MULT_X = u_params.x;
    float MULT_Y = u_params.y;
    float HUE = u_params.z; // 0 / 360
    float BRIGHTNESS = u_params.w;
    float MOUSE_BRIGHTNESS = u_params2.x; // -1 / 1 -- -1 being black and 100% power
    float SCALE = u_params2.y;
    float NOISE_FACTOR = u_params2.z;
    float BW = u_params2.w;

    // shader
    vec2 uv = FlutterFragCoord().xy / resolution.xy;
    vec2 mouse_uv = ((u_mouse / resolution.xy) - 0.5) * 2;

    vec2 scale_uv = uv;
    scale_uv -= vec2(.5);
    scale_uv *= SCALE;

    // noise
    float noise = cnoise(scale_uv, u_time) * NOISE_FACTOR;

    // mouse
    float c_d = distance(scale_uv.x, .5);
    c_d = smoothstep(0., .6, c_d);
    vec2 m_uv = scale_uv * (c_d + cos(scale_uv.x * MULT_X) * noise - sin(scale_uv.y * MULT_Y) * noise);

    // end uv deformation
    scale_uv += vec2(.5);

    // mouse 
    float dist = distance(m_uv, mouse_uv * SCALE/2.);

    dist = 1. - dist;
    dist = smoothstep(.3, 1., dist);

    // # COMPUTE
    vec3 current_color = u_color;
    vec3 col = palette(
        u_time + cos((m_uv.x) + (m_uv.y)), 
        current_color, amplitude, frequency, phase_shift, u_is_dark > .5 ? 1 : -1
    );

    if (u_is_dark > .5) {
        vec3 shift_col = hueShift(col, u_hue_sway);

        col = mix(
            col, 
            shift_col * col + (dist * MOUSE_BRIGHTNESS), 
            dist
        );

        // // final shift
        col = hueShift(col, u_hue_turn);
        col *= BRIGHTNESS;

        float bw_col = (col.r + col.g + col.b) * .3;
        col = mix(col, vec3(bw_col), BW);
        col = vec3(1.) - col;
    } else {
        col = current_color;

        col = mix(
            col, 
            u_color2, 
            dist
        );
    }

    col = clamp(col, 0.0, 1.0);
    fragColor = vec4(col, 1.0);
}
