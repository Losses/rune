//! The shader sources of a scene: the runtime's prelude in front of the author's files, checked
//! with naga so that errors name the author's file and line.

use std::fmt::Write;

use anyhow::{Result, anyhow};

use super::inputs::{MusicValues, Word};
use super::manifest::{Manifest, PREV, ParamKind, Value};

/// The runtime's functions (`rv_*`).
const STDLIB: &str = include_str!("stdlib.wgsl");

pub const BIND_FRAME: u32 = 0;
pub const BIND_MUSIC: u32 = 1;
pub const BIND_PARAMS: u32 = 2;
pub const BIND_STATE: u32 = 3;
pub const BIND_POST: u32 = 4;
pub const BIND_SPECTRUM: u32 = 5;
pub const BIND_WAVEFORM: u32 = 6;
pub const BIND_BLUE_NOISE: u32 = 7;
pub const BIND_LINEAR_CLAMP: u32 = 8;
pub const BIND_LINEAR_REPEAT: u32 = 9;
pub const BIND_NEAREST_CLAMP: u32 = 10;
pub const BIND_ASSETS: u32 = 11;

/// `struct Frame` is 48 bytes: time, dt, index, first, resolution (vec2f), aspect, seed, and the
/// app's accent colour (vec4f) at 32.
pub const FRAME_SIZE: u64 = 48;
/// `struct Post` is 64 bytes: eight f32, the tint (vec4f) at 32, contrast and saturation, padding.
pub const POST_SIZE: u64 = 64;

pub const FRAME: &str = "struct Frame {
  time: f32,
  dt: f32,
  index: u32,
  first: u32,
  resolution: vec2f,
  aspect: f32,
  seed: u32,
  accent: vec4f,
}
";

pub const POST: &str = "struct Post {
  exposure: f32,
  bloom_intensity: f32,
  bloom_threshold: f32,
  bloom_radius: f32,
  chromatic: f32,
  vignette: f32,
  grain: f32,
  flash: f32,
  tint: vec4f,
  contrast: f32,
  saturation: f32,
}
";

const PIXEL: &str = "struct Pixel {
  uv: vec2f,
  p: vec2f,
  coord: vec2f,
  size: vec2f,
}
";

/// What stands in for `struct State` when a scene keeps none.
const NO_STATE: &str = "struct State {\n  rv_unused: u32,\n}\n";

/// The layout of `struct Params`: the WGSL, its size and each parameter's offset. Scalars take 4
/// bytes, colours a 16-byte aligned vec4f; the size is a multiple of 16 as uniforms want.
pub struct ParamsLayout {
    pub wgsl: String,
    pub size: u64,
    pub offsets: Vec<u64>,
}

impl ParamsLayout {
    pub fn new(manifest: &Manifest) -> Self {
        let mut wgsl = String::from("struct Params {\n");
        let mut offset = 0u64;
        let mut offsets = Vec::new();
        for param in &manifest.params {
            let ty = match param.kind {
                ParamKind::Float => "f32",
                ParamKind::Int => "i32",
                ParamKind::Bool | ParamKind::Choice => "u32",
                ParamKind::Color => "vec4f",
            };
            if param.kind == ParamKind::Color {
                offset = offset.next_multiple_of(16);
            }
            offsets.push(offset);
            offset += if param.kind == ParamKind::Color {
                16
            } else {
                4
            };
            let _ = writeln!(wgsl, "  {}: {ty},", param.id);
        }
        if manifest.params.is_empty() {
            wgsl.push_str("  rv_unused: u32,\n");
            offset = 4;
        }
        wgsl.push_str("}\n");
        Self {
            wgsl,
            size: offset.next_multiple_of(16),
            offsets,
        }
    }

    /// The bytes of `values`, one per parameter in the manifest's order.
    pub fn pack(&self, values: &[Value]) -> Vec<u8> {
        let mut bytes = vec![0u8; self.size as usize];
        for (value, &offset) in values.iter().zip(&self.offsets) {
            let at = offset as usize;
            match *value {
                Value::Float(v) => bytes[at..at + 4].copy_from_slice(&v.to_le_bytes()),
                Value::Int(v) => bytes[at..at + 4].copy_from_slice(&v.to_le_bytes()),
                Value::Uint(v) => bytes[at..at + 4].copy_from_slice(&v.to_le_bytes()),
                Value::Color(rgb) => {
                    for (i, c) in rgb.iter().chain(&[1.0]).enumerate() {
                        bytes[at + i * 4..at + i * 4 + 4].copy_from_slice(&c.to_le_bytes());
                    }
                }
            }
        }
        bytes
    }
}

fn music_struct() -> String {
    let mut wgsl = String::from("struct Music {\n");
    for (name, word) in MusicValues::default().fields() {
        let ty = match word {
            Word::F(_) => "f32",
            Word::U(_) => "u32",
        };
        let _ = writeln!(wgsl, "  {name}: {ty},");
    }
    wgsl.push_str("}\n");
    wgsl
}

/// Which module a source is for.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Update,
    Pass,
}

/// The declarations every module starts with.
pub fn prelude(manifest: &Manifest, stage: Stage) -> String {
    let mut s = String::new();
    s.push_str(FRAME);
    s.push_str(&music_struct());
    s.push_str(POST);
    s.push_str(PIXEL);
    s.push_str(&ParamsLayout::new(manifest).wgsl);
    let state_access = if stage == Stage::Update {
        "read_write"
    } else {
        "read"
    };
    let _ = write!(
        s,
        "@group(0) @binding({BIND_FRAME}) var<uniform> frame: Frame;
@group(0) @binding({BIND_MUSIC}) var<uniform> music: Music;
@group(0) @binding({BIND_PARAMS}) var<uniform> params: Params;
@group(0) @binding({BIND_STATE}) var<storage, {state_access}> state: State;
@group(0) @binding({BIND_SPECTRUM}) var spectrum: texture_2d<f32>;
@group(0) @binding({BIND_WAVEFORM}) var waveform: texture_2d<f32>;
@group(0) @binding({BIND_BLUE_NOISE}) var blue_noise: texture_2d<f32>;
@group(0) @binding({BIND_LINEAR_CLAMP}) var linear_clamp: sampler;
@group(0) @binding({BIND_LINEAR_REPEAT}) var linear_repeat: sampler;
@group(0) @binding({BIND_NEAREST_CLAMP}) var nearest_clamp: sampler;
"
    );
    if stage == Stage::Update {
        let _ = writeln!(
            s,
            "@group(0) @binding({BIND_POST}) var<storage, read_write> post: Post;"
        );
    }
    for (i, asset) in manifest.assets.iter().enumerate() {
        let _ = writeln!(
            s,
            "@group(0) @binding({}) var asset_{}: texture_2d<f32>;",
            BIND_ASSETS + i as u32,
            asset.id
        );
    }
    s.push_str(
        "// The spectrum at x (0..1, 30 Hz to 16 kHz on a log scale), age rows back (0 is now).
fn rv_spectrum(x: f32, age: f32) -> f32 {
  let size = vec2f(textureDimensions(spectrum));
  return textureSampleLevel(spectrum, linear_clamp, vec2f(x, (age + 0.5) / size.y), 0.0).r;
}

// Stereo samples of the latest waveform, x 0..1 from oldest to newest.
fn rv_waveform(x: f32) -> vec2f {
  return textureSampleLevel(waveform, linear_clamp, vec2f(x, 0.5), 0.0).rg;
}

// Blue noise at a pixel, 0..1 per channel; a different pattern each frame when animated.
fn rv_blue_noise(coord: vec2f, animated: bool) -> vec4f {
  var c = vec2i(coord);
  if (animated) {
    c += vec2i(i32(rv_hash(frame.seed) & 63u), i32(rv_hash(frame.seed + 1u) & 63u));
  }
  return textureLoad(blue_noise, c & vec2i(63), 0);
}
",
    );
    s.push_str(STDLIB);
    s
}

/// The name a pass's input has in WGSL: `prev:x` is `prev_x`.
pub fn input_name(input: &str) -> String {
    match input.strip_prefix(PREV) {
        Some(id) => format!("prev_{id}"),
        None => input.to_string(),
    }
}

/// Pieces of WGSL put together, remembering which file each line came from.
#[derive(Default)]
pub struct Source {
    pub text: String,
    /// (first line, 1-based; file name) of each piece.
    pieces: Vec<(u32, String)>,
    lines: u32,
}

impl Source {
    pub fn push(&mut self, name: &str, text: &str) {
        self.pieces.push((self.lines + 1, name.to_string()));
        self.text.push_str(text);
        if !text.ends_with('\n') {
            self.text.push('\n');
        }
        self.lines += text.lines().count().max(1) as u32;
    }

    /// The file and the line in it of a line of the whole.
    fn locate(&self, line: u32) -> (&str, u32) {
        let (start, name) = self
            .pieces
            .iter()
            .rev()
            .find(|(start, _)| *start <= line)
            .map_or((1, "?"), |(s, n)| (*s, n.as_str()));
        (name, line - start + 1)
    }

    /// `file:line:column: message`.
    fn at(&self, location: Option<naga::SourceLocation>, message: &str) -> String {
        match location {
            Some(l) => {
                let (name, line) = self.locate(l.line_number);
                format!("{name}:{line}:{}: {message}", l.line_position)
            }
            None => message.to_string(),
        }
    }

    /// Parse and validate, with errors in the author's terms.
    pub fn check(&self) -> Result<naga::Module> {
        let module = naga::front::wgsl::parse_str(&self.text)
            .map_err(|e| anyhow!(self.at(e.location(&self.text), &parse_message(&e))))?;
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::default(),
        );
        if let Err(e) = validator.validate(&module) {
            let mut message = e.as_inner().to_string();
            let mut source = std::error::Error::source(e.as_inner());
            while let Some(inner) = source {
                let _ = write!(message, ": {inner}");
                source = inner.source();
            }
            // The last span is the innermost: the expression rather than its function.
            let location = e.spans().last().map(|(span, _)| span.location(&self.text));
            return Err(anyhow!(self.at(location, &message)));
        }
        Ok(module)
    }
}

/// naga's message with the labels of its spans ("expected ...").
fn parse_message(e: &naga::front::wgsl::ParseError) -> String {
    let mut message = e.message().to_string();
    for (_, label) in e.labels() {
        if !label.is_empty() {
            let _ = write!(message, " ({label})");
        }
    }
    message
}

/// The size of `struct State` in `module` (at least 16 bytes).
pub fn state_size(module: &naga::Module) -> Result<u64> {
    let mut layouter = naga::proc::Layouter::default();
    layouter
        .update(module.to_ctx())
        .map_err(|e| anyhow!("State: {e}"))?;
    let handle = module
        .types
        .iter()
        .find(|(_, ty)| ty.name.as_deref() == Some("State"))
        .map(|(handle, _)| handle)
        .ok_or_else(|| anyhow!("no struct State"))?;
    Ok(u64::from(layouter[handle].size).max(16))
}

/// The struct State the author declares, or the runtime's stand-in.
pub fn state_text(state: Option<&str>) -> &str {
    state.unwrap_or(NO_STATE)
}

/// The update module: prelude, state and update files, and the entry point.
pub fn update_source(manifest: &Manifest, state: (&str, &str), update: (&str, &str)) -> Source {
    let mut source = Source::default();
    source.push("<runtime>", &prelude(manifest, Stage::Update));
    source.push(state.0, state.1);
    source.push(update.0, update.1);
    source.push(
        "<runtime>",
        "@compute @workgroup_size(1)\nfn rv_update() {\n  update();\n}\n",
    );
    source
}

/// A pass's module: prelude, state file, the common files, its inputs, its file and the entry
/// points. `scale` is the target's size against the internal resolution.
pub fn pass_source(
    manifest: &Manifest,
    state: (&str, &str),
    common: &[(String, String)],
    inputs: &[String],
    shader: (&str, &str),
    scale: f32,
) -> Source {
    let mut source = Source::default();
    source.push("<runtime>", &prelude(manifest, Stage::Pass));
    source.push(state.0, state.1);
    for (name, text) in common {
        source.push(name, text);
    }
    let mut bindings = String::new();
    for (i, input) in inputs.iter().enumerate() {
        let _ = writeln!(
            bindings,
            "@group(1) @binding({i}) var {}: texture_2d<f32>;",
            input_name(input)
        );
    }
    source.push("<runtime>", &bindings);
    source.push(shader.0, shader.1);
    source.push(
        "<runtime>",
        &format!(
            "struct RvVaryings {{
  @builtin(position) position: vec4f,
}}

@vertex
fn rv_vertex(@builtin(vertex_index) i: u32) -> RvVaryings {{
  let xy = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
  var out: RvVaryings;
  out.position = vec4f(xy * 2.0 - 1.0, 0.0, 1.0);
  return out;
}}

@fragment
fn rv_fragment(v: RvVaryings) -> @location(0) vec4f {{
  var px: Pixel;
  px.size = max(floor(frame.resolution * {scale:?}), vec2f(1.0));
  px.coord = v.position.xy;
  px.uv = px.coord / px.size;
  px.p = vec2f(px.uv.x - 0.5, 0.5 - px.uv.y) * vec2f(px.size.x / px.size.y, 1.0);
  return shade(px);
}}
"
        ),
    );
    source
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        Manifest::parse(
            r##"
format = 1
name = "T"
license = "MPL-2.0"
[[param]]
id = "speed"
default = 1.5
[[param]]
id = "tint"
type = "color"
default = "#ffffff"
[[param]]
id = "on"
type = "bool"
default = true
[[buffer]]
id = "trail"
persistent = true
[[pass]]
shader = "a.wgsl"
target = "out"
inputs = ["prev:trail"]
"##,
        )
        .unwrap()
    }

    const STATE: &str = "struct State {\n  eye: vec3f,\n  kick: f32,\n  count: u32,\n}\n";

    #[test]
    fn params_are_laid_out_as_wgsl_wants() {
        let m = manifest();
        let layout = ParamsLayout::new(&m);
        assert_eq!(layout.offsets, vec![0, 16, 32]);
        assert_eq!(layout.size, 48);
        let values: Vec<Value> = m.params.iter().map(|p| p.value().unwrap()).collect();
        let bytes = layout.pack(&values);
        assert_eq!(&bytes[0..4], &1.5f32.to_le_bytes());
        assert_eq!(&bytes[28..32], &1.0f32.to_le_bytes());
        assert_eq!(&bytes[32..36], &1u32.to_le_bytes());
    }

    #[test]
    fn modules_validate_and_report_the_state_size() {
        let m = manifest();
        let update = update_source(
            &m,
            ("state.wgsl", STATE),
            (
                "update.wgsl",
                "fn update() {\n  state.kick = music.low * params.speed;\n  post.flash = 0.0;\n}\n",
            ),
        );
        let module = update.check().unwrap();
        assert_eq!(state_size(&module).unwrap(), 32);
        let pass = pass_source(
            &m,
            ("state.wgsl", STATE),
            &[],
            &["prev:trail".to_string()],
            (
                "a.wgsl",
                "fn shade(px: Pixel) -> vec4f {\n  let t = textureSampleLevel(prev_trail, linear_clamp, px.uv, 0.0);\n  return t + vec4f(params.tint.rgb * state.kick, 1.0) * rv_spectrum(px.uv.x, 0.0);\n}\n",
            ),
            1.0,
        );
        pass.check().unwrap();
    }

    #[test]
    fn errors_point_into_the_authors_file() {
        let m = manifest();
        let pass = pass_source(
            &m,
            ("state.wgsl", STATE),
            &[],
            &[],
            (
                "a.wgsl",
                "fn shade(px: Pixel) -> vec4f {\n  let x = 1.0;\n  return vec4f(y);\n}\n",
            ),
            1.0,
        );
        let message = format!("{:#}", pass.check().unwrap_err());
        assert!(message.starts_with("a.wgsl:3:"), "{message}");
        // Writing the state is only for update().
        let pass = pass_source(
            &m,
            ("state.wgsl", STATE),
            &[],
            &[],
            (
                "a.wgsl",
                "fn shade(px: Pixel) -> vec4f {\n  state.kick = 1.0;\n  return vec4f(0.0);\n}\n",
            ),
            1.0,
        );
        let message = format!("{:#}", pass.check().unwrap_err());
        assert!(message.starts_with("a.wgsl:2:"), "{message}");
    }

    #[test]
    fn the_library_common_files_and_a_scene_without_state_validate() {
        let m = manifest();
        let pass = pass_source(
            &m,
            ("<runtime>", state_text(None)),
            &[(
                "lib.wgsl".to_string(),
                "fn twice(x: f32) -> f32 {\n  return 2.0 * x;\n}\n".to_string(),
            )],
            &["prev:trail".to_string()],
            (
                "a.wgsl",
                "fn shade(px: Pixel) -> vec4f {\n  return vec4f(twice(rv_fbm(px.p, 4)), rv_hue_rotate(vec3f(1.0, 0.0, 0.0), 1.0).g, rv_blue_noise(px.coord, true).r, 1.0);\n}\n",
            ),
            0.5,
        );
        pass.check().unwrap();
    }
}
