#version 460
#include <flutter/runtime_effect.glsl>

out vec4 fragColor;

uniform vec2 resolution;
uniform sampler2D image;

uniform vec2 u_res;
uniform float u_time;
uniform vec2 u_mouse;
uniform vec4 u_params;
uniform vec4 u_params2;
uniform vec4 u_altparams;
uniform vec3 u_color;
uniform vec3 u_color2;
uniform float u_mode;
uniform float u_swap;

const float MPI = 6.28318530718;

// cos mix
vec3 palette(in float t, in vec3 a, in vec3 b, in vec3 c, in vec3 d) {
    return a + b * cos(6.28318 * (c * t + d));
}

// hue shift
vec3 hueShift(vec3 color, float hueAdjust) {
    const vec3 kRGBToYPrime = vec3(0.299, 0.587, 0.114);
    const vec3 kRGBToI = vec3(0.596, -0.275, -0.321);
    const vec3 kRGBToQ = vec3(0.212, -0.523, 0.311);
    const vec3 kYIQToR = vec3(1.0, 0.956, 0.621);
    const vec3 kYIQToG = vec3(1.0, -0.272, -0.647);
    const vec3 kYIQToB = vec3(1.0, -1.107, 1.704);

    float YPrime = dot(color, kRGBToYPrime);
    float I = dot(color, kRGBToI);
    float Q = dot(color, kRGBToQ);
    float hue = atan(Q, I);
    float chroma = sqrt(I * I + Q * Q);

    hue += hueAdjust;

    Q = chroma * sin(hue);
    I = chroma * cos(hue);

    vec3 yIQ = vec3(YPrime, I, Q);

    return vec3(dot(yIQ, kYIQToR), dot(yIQ, kYIQToG), dot(yIQ, kYIQToB));
}

// noise
vec4 permute(vec4 x) { return mod(((x * 34.0) + 1.0) * x, 289.0); }
vec4 taylorInvSqrt(vec4 r) { return 1.79284291400159 - 0.85373472095314 * r; }
vec3 fade(vec3 t) { return t * t * t * (t * (t * 6.0 - 15.0) + 10.0); }

float cnoise(vec3 P) {
    vec3 Pi0 = floor(P);
    vec3 Pi1 = Pi0 + vec3(1.0);
    Pi0 = mod(Pi0, 289.0);
    Pi1 = mod(Pi1, 289.0);
    vec3 Pf0 = fract(P);
    vec3 Pf1 = Pf0 - vec3(1.0);
    vec4 ix = vec4(Pi0.x, Pi1.x, Pi0.x, Pi1.x);
    vec4 iy = vec4(Pi0.yy, Pi1.yy);
    vec4 iz0 = Pi0.zzzz;
    vec4 iz1 = Pi1.zzzz;

    vec4 ixy = permute(permute(ix) + iy);
    vec4 ixy0 = permute(ixy + iz0);
    vec4 ixy1 = permute(ixy + iz1);

    vec4 gx0 = ixy0 / 7.0;
    vec4 gy0 = fract(floor(gx0) / 7.0) - 0.5;
    gx0 = fract(gx0);
    vec4 gz0 = vec4(0.5) - abs(gx0) - abs(gy0);
    vec4 sz0 = step(gz0, vec4(0.0));
    gx0 -= sz0 * (step(0.0, gx0) - 0.5);
    gy0 -= sz0 * (step(0.0, gy0) - 0.5);

    vec4 gx1 = ixy1 / 7.0;
    vec4 gy1 = fract(floor(gx1) / 7.0) - 0.5;
    gx1 = fract(gx1);
    vec4 gz1 = vec4(0.5) - abs(gx1) - abs(gy1);
    vec4 sz1 = step(gz1, vec4(0.0));
    gx1 -= sz1 * (step(0.0, gx1) - 0.5);
    gy1 -= sz1 * (step(0.0, gy1) - 0.5);

    vec3 g000 = vec3(gx0.x, gy0.x, gz0.x);
    vec3 g100 = vec3(gx0.y, gy0.y, gz0.y);
    vec3 g010 = vec3(gx0.z, gy0.z, gz0.z);
    vec3 g110 = vec3(gx0.w, gy0.w, gz0.w);
    vec3 g001 = vec3(gx1.x, gy1.x, gz1.x);
    vec3 g101 = vec3(gx1.y, gy1.y, gz1.y);
    vec3 g011 = vec3(gx1.z, gy1.z, gz1.z);
    vec3 g111 = vec3(gx1.w, gy1.w, gz1.w);

    vec4 norm0 = taylorInvSqrt(vec4(dot(g000, g000), dot(g010, g010), dot(g100, g100), dot(g110, g110)));
    g000 *= norm0.x;
    g010 *= norm0.y;
    g100 *= norm0.z;
    g110 *= norm0.w;
    vec4 norm1 = taylorInvSqrt(vec4(dot(g001, g001), dot(g011, g011), dot(g101, g101), dot(g111, g111)));
    g001 *= norm1.x;
    g011 *= norm1.y;
    g101 *= norm1.z;
    g111 *= norm1.w;

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

// colors
const vec3 col1 = vec3(0.5, 0.5, 0.5);
const vec3 col2 = vec3(0.5, 0.5, 0.5);
const vec3 col3 = vec3(1.0, 1.0, 1.0);

void main() {
    vec2 uv = gl_FragCoord.xy / u_res;
    vec2 scale_uv = uv;
    scale_uv -= vec2(.5);
    scale_uv *= mix(u_params2.y, u_altparams.x, u_swap);

    float noise = cnoise(vec3(scale_uv, u_time)) * u_params2.z;

    float c_d = distance(scale_uv.x, .5);
    c_d = smoothstep(0., .6, c_d);
    vec2 m_uv = scale_uv * (c_d + cos(scale_uv.x * u_params.x) * noise - sin(scale_uv.y * u_params.y) * noise);

    scale_uv += vec2(.5);

    vec3 current_color = mix(u_color, u_color2, u_swap);
    vec3 col = palette(
        u_time + cos((m_uv.x) + (m_uv.y)),
        col1, col2, col2, current_color
    );

    float dist = distance(m_uv, u_mouse * mix(u_params2.y, u_altparams.x, u_swap) / 2.);
    dist = 1. - dist;
    dist = smoothstep(.3, 1., dist);

    vec3 shift_col = hueShift(col, sin(u_time * MPI));

    col = mix(
        col,
        shift_col * col + (dist * u_params2.x),
        dist
    );

    col = hueShift(col, u_params.z);
    col *= u_params.w;

    float bw_col = (col.r + col.g + col.b) * .3;
    col = mix(col, vec3(bw_col), mix(u_params2.w, u_altparams.y, u_swap));

    if (u_mode > .5) {
        col = vec3(1.) - col;
    }

    vec2 fragCoord = (FlutterFragCoord().xy / resolution.xy);
    vec3 imageColor = texture(image, fragCoord).xyz;
    float luminance = imageColor.r * 0.299 + imageColor.g * 0.587 + imageColor.b * 0.114;
    vec3 grayColor = vec3(luminance);

    // fragColor = vec4(grayColor * col, 1.0);
    fragColor = vec4(grayColor, 1.0);
}
