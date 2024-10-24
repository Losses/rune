#version 460
#include <flutter/runtime_effect.glsl>

out vec4 fragColor;

uniform sampler2D image;

uniform vec2 resolution;
uniform float u_alpha;

uniform float u_time;
uniform vec2 u_mouse;

float noise(vec2 t) {
    return fract(sin(dot(t.xy, vec2(12.9898,78.233))) * 43758.5453);
}

vec4 lensflare(vec2 uv, vec2 pos) {
    vec2 main = uv - pos;

    vec2 uvd = uv * length(uv);

    uvd *= 8;
    pos *= 8;

    float f0 = 1.00 / (length(main) * 200.0 + 1.0);

    float f2r = max(1.00 / (1.0 + 32.0 * pow(length(uvd + 0.800 * pos), 2.0)), .0) * 0.25;
    float f2g = max(1.00 / (1.0 + 32.0 * pow(length(uvd + 0.850 * pos), 2.0)), .0) * 0.23;
    float f2b = max(1.00 / (1.0 + 32.0 * pow(length(uvd + 0.890 * pos), 2.0)), .0) * 0.21;

    vec2 uvx = mix(uv, uvd, -0.5);

    float f4r = max(0.01 - pow(length(uvx + 0.400 * pos), 2.4), .0) * 6.00;
    float f4g = max(0.01 - pow(length(uvx + 0.450 * pos), 2.4), .0) * 5.00;
    float f4b = max(0.01 - pow(length(uvx + 0.490 * pos), 2.4), .0) * 3.00;

    uvx = mix(uv, uvd, -0.4);

    float f5r = max(0.01 - pow(length(uvx + 0.200 * pos), 5.5), .0) * 2.00;
    float f5g = max(0.01 - pow(length(uvx + 0.400 * pos), 5.5), .0) * 2.00;
    float f5b = max(0.01 - pow(length(uvx + 0.600 * pos), 5.5), .0) * 2.00;

    uvx = mix(uv, uvd, -0.5);

    float f6r = max(0.01 - pow(length(uvx - 0.300 * pos), 1.6), .0) * 6.00;
    float f6g = max(0.01 - pow(length(uvx - 0.325 * pos), 1.6), .0) * 3.00;
    float f6b = max(0.01 - pow(length(uvx - 0.350 * pos), 1.6), .0) * 5.00;

    vec3 c = vec3(.0);

    c.r += f2r + f4r + f5r + f6r;
    c.g += f2g + f4g + f5g + f6g;
    c.b += f2b + f4b + f5b + f6b;
    c = c * 1.3 - vec3(length(uvd) * .05);
    c += vec3(f0);

    float alpha = max(c.r, max(c.g, c.b));

    return vec4(c, alpha);
}

vec4 cc(vec4 color, float factor, float factor2) {
    vec3 c = color.xyz;
    float w = color.x + color.y + color.z;
    return vec4(mix(c, vec3(w) * factor, w * factor2), color.a);
}

void main() {
    vec2 coord = FlutterFragCoord();

    vec2 uv = coord / resolution.xy;
    uv -= 0.5;
    uv.x *= resolution.x / resolution.y;

    vec2 mouse = u_mouse.xy / resolution.xy;
    mouse -= 0.5;
    mouse.x *= resolution.x / resolution.y;

    vec4 flareColor = vec4(1.4, 1.2, 1.0, 1.0) * lensflare(uv, mouse);

    float n = noise(coord) * 0.015;
    flareColor -= vec4(n, n, n, 0.0);
    flareColor = cc(flareColor, 0.5, 0.1);

    vec2 fragCoord = (FlutterFragCoord().xy / resolution.xy);
    vec4 imageColor = texture(image, fragCoord);

    fragColor = flareColor * u_alpha + imageColor;
}