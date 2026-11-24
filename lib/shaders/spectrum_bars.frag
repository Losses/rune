#version 460
#include <flutter/runtime_effect.glsl>

precision highp float;

// The first of the two passes of the spectrum's glow, the bars under a
// Gaussian blur; spectrum_glow.frag is the second. A Gaussian blur is a blur
// up and down followed by one across. This pass does the first, exactly: up
// and down, the blur of a bar is the difference of two normal distribution
// functions. It works the blur out at the middle of each column of an image a
// few logical pixels wide, with each bar shared between the columns whose
// middles are nearer to it than a column's width, in proportion to how near
// (a tent two columns wide). That keeps each bar's weight and centre, and the
// second pass blurs the columns across.
//
// The glow used to be worked out in one pass, which summed every bar within 3
// deviations of a pixel: 59 of the player's 512 bars on a screen 640 pixels
// wide. A column takes the few bars under its tent.
//
// Before the shaders, the bars were lines drawn one after another under a
// blur filter. Under Skia, two lines that share a pixel each cover part of it,
// and blending the second over the first leaves it short by the product of
// the two parts: a seam. This pass takes those seams out again, so that the
// glow is as bright as it was. Where a seam falls in its pixel moves with the
// screen's width, its pixel ratio and the window's place; on average over
// those it takes a sixth of a pixel across, for as far up and down as both
// bars reach.

out vec4 fragColor;

// The bars' half heights, four to an entry: bar 4 * j + k is component k of
// entry j. The player sends 512 bands (playback/src/internal.rs). GLES gives
// each element of a uniform array a slot of four floats and guarantees 224
// slots from version 3.0 on: 512 single floats would not fit, 128 groups of
// four do. The array comes first, where every backend aligns it alike.
uniform vec4 u_half_heights[128];
// Bar i is centred at x = i * u_bar_width, is u_bar_width wide, and reaches
// from the middle line y = u_middle up and down by its half height.
uniform float u_bar_width;
uniform float u_middle;
uniform float u_count;
// The standard deviation of the blur, in pixels.
uniform float u_sigma;
// The width of a column, in pixels.
uniform float u_column;
// The width of the seam between two bars, in pixels: a sixth of a pixel of
// the screen under Skia, and none under Impeller, which drew the lines whole.
uniform float u_seam;

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

// The share of the tent max(0, 1 - |s|), whose area is 1, that lies below s.
float tentBelow(float s) {
  float c = clamp(s, -1.0, 1.0);
  return c < 0.0 ? 0.5 * (1.0 + c) * (1.0 + c)
                 : 1.0 - 0.5 * (1.0 - c) * (1.0 - c);
}

// The share of a bar reaching halfHeight up and down from the middle line,
// blurred up and down by u_sigma, at fromMiddle from that line.
float along(float halfHeight, float fromMiddle) {
  return below((halfHeight - fromMiddle) / u_sigma)
      - below((-halfHeight - fromMiddle) / u_sigma);
}

// The share of the tent of the column at x that lies between from and to.
float under(float from, float to, float x) {
  return tentBelow((to - x) / u_column) - tentBelow((from - x) / u_column);
}

void main() {
  vec2 p = FlutterFragCoord().xy;
  // Further than 3 deviations from the middle line beyond a bar's end, the
  // bar adds less than 0.14% of its glow.
  float reach = 3.0 * u_sigma;
  // The bars reach as far down from the middle line as up.
  float fromMiddle = abs(p.y - u_middle);
  // The bars under the tent, and those whose seams can reach under it.
  float from = p.x - u_column - u_seam;
  float to = p.x + u_column + u_seam;
  float sum = 0.0;
  // The half height of the bar before, while there is one.
  float previous = -1.0;

  // The bars are in 16 runs of 32, 8 entries each: a run off the tent is
  // passed over whole, so a column looks at no more than a few entries.
  for (int run = 0; run < 16; run++) {
    float runFirst = 32.0 * float(run);
    if (runFirst >= u_count) {
      break;
    }
    float runLeft = (runFirst - 0.5) * u_bar_width;
    if (runLeft >= to) {
      break;
    }
    if (runLeft + 32.0 * u_bar_width <= from) {
      continue;
    }

    for (int entry = 0; entry < 8; entry++) {
      float first = runFirst + 4.0 * float(entry);
      float groupLeft = (first - 0.5) * u_bar_width;
      if (first >= u_count || groupLeft >= to) {
        break;
      }
      if (groupLeft + 4.0 * u_bar_width <= from) {
        continue;
      }
      vec4 halfHeights = u_half_heights[run * 8 + entry];

      for (int k = 0; k < 4; k++) {
        float i = first + float(k);
        float left = (i - 0.5) * u_bar_width;
        float right = left + u_bar_width;
        if (i >= u_count || left >= to) {
          break;
        }
        if (right <= from) {
          continue;
        }
        float h = halfHeights[k];
        if (fromMiddle < h + reach) {
          sum += under(left, right, p.x) * along(h, fromMiddle);
          // The seam with the bar before reaches as far as the shorter of
          // the two. The first bar looked at has none under the tent: the
          // bar before it, if there is one, ends a seam's width outside.
          float seam = min(previous, h);
          if (u_seam > 0.0 && previous >= 0.0 && fromMiddle < seam + reach) {
            sum -= under(left - 0.5 * u_seam, left + 0.5 * u_seam, p.x)
                * along(seam, fromMiddle);
          }
        }
        previous = h;
      }
    }
  }

  // The bars cover no more than the whole column, so the sum is at most 1,
  // and the second pass reads it from any channel.
  fragColor = vec4(max(sum, 0.0));
}
