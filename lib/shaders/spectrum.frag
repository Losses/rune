#version 460
#include <flutter/runtime_effect.glsl>

precision highp float;

// The spectrum's glow: solid vertical bars, Gaussian blurred, faded. It used
// to be drawn as bars into a layer that a blur filter and an opacity layer
// then went over every frame; this works out the same blur for each pixel.
//
// A Gaussian blur of a rectangle is the product of two one-dimensional ones,
// and each of those is the difference of two normal distribution functions,
// so the glow at a pixel is a sum over the bars within 3 deviations of it.

out vec4 fragColor;

// Bar i is centred at x = i * u_bar_width, is u_bar_width wide, and reaches
// from the middle line y = u_middle up and down by its half height.
uniform float u_bar_width;
uniform float u_middle;
uniform float u_count;
// The standard deviation of the blur, in pixels.
uniform float u_sigma;
// The bars' colour, premultiplied, with the opacity of the glow.
uniform vec4 u_color;
// The bars' half heights, four to an entry: bar 4 * j + k is component k of
// entry j. The player sends 512 bands (playback/src/internal.rs). GLES gives
// each element of a uniform array a slot of four floats and guarantees 224
// slots from version 3.0 on: 512 single floats would not fit, 128 groups of
// four do.
uniform vec4 u_half_heights[128];

// erf(x), Abramowitz and Stegun 7.1.26, error below 1.5e-7.
float erf_approx(float x) {
  float s = sign(x);
  float a = abs(x);
  float t = 1.0 / (1.0 + 0.3275911 * a);
  float y = 1.0 - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t
      - 0.284496736) * t + 0.254829592) * t * exp(-a * a);
  return s * y;
}

// The share of a Gaussian of deviation 1 that lies below z.
float below(float z) {
  return 0.5 + 0.5 * erf_approx(z * 0.70710678118);
}

void main() {
  vec2 p = FlutterFragCoord().xy;
  // Bars further than 3 deviations away add less than 0.14% of their glow,
  // and the bars beyond them on one side add no more than that together.
  float reach = 3.0 * u_sigma;
  float glow = 0.0;
  // The share below the left edge of bar i, which is the right edge of bar
  // i - 1: the bars share their edges, so each is worked out once.
  float belowLeft = -1.0;

  for (int group = 0; group < 128; group++) {
    float first = 4.0 * float(group);
    if (first >= u_count) {
      break;
    }
    // The four bars of the group lie between these: a group out of reach is
    // passed over whole.
    float groupLeft = (first - 0.5) * u_bar_width;
    if (groupLeft > p.x + reach) {
      break;
    }
    if (groupLeft + 4.0 * u_bar_width < p.x - reach) {
      continue;
    }
    vec4 halfHeights = u_half_heights[group];

    for (int k = 0; k < 4; k++) {
      float i = first + float(k);
      if (i >= u_count) {
        break;
      }
      float left = (i - 0.5) * u_bar_width;
      float right = left + u_bar_width;
      if (right < p.x - reach) {
        continue;
      }
      if (left > p.x + reach) {
        break;
      }
      float h = halfHeights[k];
      float belowRight = below((right - p.x) / u_sigma);
      if (belowLeft < 0.0) {
        belowLeft = below((left - p.x) / u_sigma);
      }
      if (abs(p.y - u_middle) < h + reach) {
        float along = below((u_middle + h - p.y) / u_sigma)
            - below((u_middle - h - p.y) / u_sigma);
        glow += (belowRight - belowLeft) * along;
      }
      belowLeft = belowRight;
    }
  }

  fragColor = u_color * glow;
}
