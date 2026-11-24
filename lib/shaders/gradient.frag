#version 460
#include <flutter/runtime_effect.glsl>

// The cover wall: the covers in grey, tinted by the colour field that
// gradient_field.frag works out at a fraction of the resolution, scaled back
// up here.

out vec4 fragColor;

uniform vec2 resolution;
// Where a point of the drawing lies in the field image: at uv * u_field_scale
// + u_field_offset. The image's first pixel sits on the drawing's edge, and it
// reaches a little past the far edge.
uniform vec2 u_field_scale;
uniform vec2 u_field_offset;
uniform float u_is_dark;

uniform sampler2D image;
uniform sampler2D field;

void main() {
    vec2 uv = FlutterFragCoord().xy / resolution.xy;
    vec3 col = texture(field, uv * u_field_scale + u_field_offset).rgb;

    vec4 imageColor = texture(image, uv);
    float luminance = imageColor.r * 0.299 + imageColor.g * 0.587 + imageColor.b * 0.114;

    if (u_is_dark > .5) {
        fragColor = vec4(vec3(luminance) * col, imageColor.a);
    } else {
        float alpha = (1.0 - luminance) * imageColor.a;
        fragColor = vec4(col * alpha, alpha);
    }
}
