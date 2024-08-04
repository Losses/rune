#version 460
#include <flutter/runtime_effect.glsl>

out vec4 fragColor;

uniform vec2 resolution;
uniform sampler2D image;
uniform sampler2D gradient;

void main() {
    vec2 uv = (FlutterFragCoord().xy / resolution.xy);
    vec3 imageColor = texture(image, uv).xyz;
    float luminance = imageColor.r * 0.299 + imageColor.g * 0.587 + imageColor.b * 0.114;
    vec3 grayColor = vec3(luminance);
    vec3 gradientColor = texture(gradient, uv).xyz;
    fragColor = vec4(grayColor * gradientColor, 1.0);
}
