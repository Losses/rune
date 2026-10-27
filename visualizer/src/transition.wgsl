// The transition pass: one fullscreen triangle that writes the output from the scene's frame
// (`fresh`) and, for a dissolve, the picture held from the scene before (`held`). `kind` picks
// the branch, `progress` runs from 0 to 1 and `color` is for the kinds that need one. Later
// kinds (a flash to the theme colour, a glitch, a mosaic) are more branches of `fragment`.
// The cut-in (`cutin.wgsl` is appended to this file's source) is one of them.

const KIND_FADE: u32 = 0u;
const KIND_DISSOLVE: u32 = 1u;
const KIND_CUTIN: u32 = 2u;

struct Transition {
  kind: u32,
  progress: f32,
  pad: vec2u,
  // A fade's colour, the theme colour of a cut-in. Display values, like the textures.
  color: vec4f,
  // A cut-in: seconds since it started, the band's opening time, its hold, its tear.
  cutin: vec4f,
  // A cut-in: the style (0 speed lines and glint, 1 halftone, 2 echo), 1 if there is a cover.
  extra: vec4f,
}

@group(0) @binding(0) var<uniform> transition: Transition;
@group(0) @binding(1) var fresh: texture_2d<f32>;
@group(0) @binding(2) var held: texture_2d<f32>;
// The cover of the song playing, as bytes.
@group(0) @binding(3) var cover: texture_2d<f32>;
@group(0) @binding(4) var cover_sampler: sampler;

@vertex
fn vertex(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
  let xy = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
  return vec4f(xy * 2.0 - 1.0, 0.0, 1.0);
}

// The frame as it is in the output's bytes: the textures are the output's format and size, so
// each pixel is read as it is, and blended as the bytes are (a fade through black is a fade of
// the displayed values).
@fragment
fn fragment(@builtin(position) position: vec4f) -> @location(0) vec4f {
  let at = vec2i(position.xy);
  let frame_now = textureLoad(fresh, at, 0);
  switch transition.kind {
    // `progress` of the way from the frame to `color`.
    case KIND_FADE: {
      return vec4f(mix(frame_now.rgb, transition.color.rgb, transition.progress), frame_now.a);
    }
    // From the held picture (0) to the frame (1).
    case KIND_DISSOLVE: {
      let old = textureLoad(held, at, 0);
      return mix(old, frame_now, transition.progress);
    }
    case KIND_CUTIN: {
      return cutin(position.xy);
    }
    default: {
      return frame_now;
    }
  }
}
