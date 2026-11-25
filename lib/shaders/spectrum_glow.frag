#version 460
#include <flutter/runtime_effect.glsl>

precision highp float;

// The second of the two passes of the spectrum's glow: the columns that
// spectrum_bars.frag blurred up and down, blurred across by a Gaussian of
// u_sigma columns, in the glow's colour.
//
// The first pass shared each bar between neighbouring columns by a tent, which
// spreads it as much as a Gaussian of 1 / sqrt(6) columns does; u_sigma leaves
// that out, so that the two together spread each bar as far as the glow's
// blur.

out vec4 fragColor;

// The glow's colour, premultiplied, with its opacity.
uniform vec4 u_color;
// The size in pixels of the image of the first pass, which this pass draws
// at its own size.
uniform vec2 u_size;
// The deviation of the blur across, in columns.
uniform float u_sigma;
// The columns on each side that the blur takes in, 3 deviations, at most 72.
uniform float u_reach;
// 1 over the sum of the weights of the taps.
uniform float u_scale;

uniform sampler2D u_columns;

void main() {
  vec2 p = FlutterFragCoord().xy;
  float sum = 0.0;
  for (int tap = 0; tap < 145; tap++) {
    float offset = float(tap) - u_reach;
    if (offset > u_reach) {
      break;
    }
    // The image is clamped at its edges, where the columns are 3 deviations
    // from the bars and hold nothing.
    float column = texture(u_columns, vec2(p.x + offset, p.y) / u_size).a;
    float d = offset / u_sigma;
    sum += exp(-0.5 * d * d) * column;
  }
  fragColor = u_color * (sum * u_scale);
}
