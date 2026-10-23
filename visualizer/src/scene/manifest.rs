//! `scene.toml`: what a scene is made of.

use std::collections::{BTreeMap, HashSet};
use std::path::{Component, Path};

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;

/// The newest format this runtime reads.
pub const FORMAT: u32 = 1;
/// The file that makes a folder a scene.
pub const MANIFEST: &str = "scene.toml";
pub const MAX_PASSES: usize = 16;
pub const MAX_BUFFERS: usize = 8;
pub const MAX_ASSETS: usize = 4;
/// The built-in HDR target the post-processing reads.
pub const OUT: &str = "out";
/// Inputs that read a persistent buffer's previous frame start with this.
pub const PREV: &str = "prev:";

/// Text for people: one string, or one per language (`zh`, `en`, ...).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Text {
    Plain(String),
    Localized(BTreeMap<String, String>),
}

impl Text {
    /// The text in `lang`, else English, else any.
    #[cfg_attr(not(test), allow(dead_code, reason = "for the tuning page"))]
    pub fn get(&self, lang: &str) -> &str {
        match self {
            Self::Plain(text) => text,
            Self::Localized(map) => map
                .get(lang)
                .or_else(|| map.get("en"))
                .or_else(|| map.values().next())
                .map_or("", String::as_str),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub name: String,
    #[serde(default)]
    pub author: Option<String>,
    pub license: String,
    #[serde(default)]
    pub description: Option<Text>,
    #[serde(default)]
    pub render: Render,
    #[serde(default)]
    pub state: Option<StateFiles>,
    /// WGSL files that every pass includes (the update does not): code the passes share.
    #[serde(default)]
    pub common: Vec<String>,
    #[serde(default, rename = "param")]
    pub params: Vec<Param>,
    #[serde(default, rename = "buffer")]
    pub buffers: Vec<Buffer>,
    #[serde(default, rename = "pass")]
    pub passes: Vec<Pass>,
    #[serde(default)]
    pub post: PostDefaults,
    #[serde(default, rename = "asset")]
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Render {
    /// The internal resolution against the output's.
    pub scale: f32,
}

impl Default for Render {
    fn default() -> Self {
        Self { scale: 1.0 }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct StateFiles {
    /// Declares `struct State`; part of every shader.
    pub file: String,
    /// Declares `fn update()`, run once per frame.
    #[serde(default)]
    pub update: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParamKind {
    #[default]
    Float,
    Int,
    Bool,
    Color,
    Choice,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Param {
    pub id: String,
    #[serde(default, rename = "type")]
    pub kind: ParamKind,
    #[serde(default)]
    pub group: Option<Text>,
    #[serde(default)]
    pub label: Option<Text>,
    #[serde(default)]
    pub unit: Option<String>,
    pub default: toml::Value,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub step: Option<f64>,
    /// The choices of a `choice`; its value is the index.
    #[serde(default)]
    pub options: Vec<Text>,
}

/// A parameter's value, as the shaders see it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    Float(f32),
    Int(i32),
    /// `bool` and `choice`.
    Uint(u32),
    /// Linear RGB.
    Color([f32; 3]),
}

impl Param {
    /// The default value, checked against the kind.
    pub fn value(&self) -> Result<Value> {
        let id = &self.id;
        Ok(match (self.kind, &self.default) {
            (ParamKind::Float, toml::Value::Float(v)) => Value::Float(*v as f32),
            (ParamKind::Float, toml::Value::Integer(v)) => Value::Float(*v as f32),
            (ParamKind::Int, toml::Value::Integer(v)) => {
                Value::Int(i32::try_from(*v).with_context(|| format!("{id}: out of range"))?)
            }
            (ParamKind::Bool, toml::Value::Boolean(v)) => Value::Uint(u32::from(*v)),
            (ParamKind::Choice, toml::Value::Integer(v)) => {
                let index = usize::try_from(*v).ok().filter(|i| *i < self.options.len());
                Value::Uint(index.with_context(|| format!("{id}: no option {v}"))? as u32)
            }
            (ParamKind::Color, toml::Value::String(v)) => Value::Color(parse_color(v)?),
            (kind, value) => bail!("{id}: {value} is not a {kind:?} value"),
        })
    }
}

/// `#rrggbb` (sRGB) as linear RGB.
pub fn parse_color(text: &str) -> Result<[f32; 3]> {
    let hex = text
        .strip_prefix('#')
        .filter(|h| h.len() == 6 && h.is_ascii())
        .with_context(|| format!("{text:?} is not a #rrggbb colour"))?;
    let mut rgb = [0.0; 3];
    for (i, c) in rgb.iter_mut().enumerate() {
        let byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
            .with_context(|| format!("{text:?} is not a #rrggbb colour"))?;
        *c = srgb_to_linear(f32::from(byte) / 255.0);
    }
    Ok(rgb)
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    #[default]
    Rgba16float,
    Rgba32float,
    Rgba8unorm,
    R16float,
    Rg16float,
}

impl Format {
    pub fn wgpu(self) -> wgpu::TextureFormat {
        match self {
            Self::Rgba16float => wgpu::TextureFormat::Rgba16Float,
            Self::Rgba32float => wgpu::TextureFormat::Rgba32Float,
            Self::Rgba8unorm => wgpu::TextureFormat::Rgba8Unorm,
            Self::R16float => wgpu::TextureFormat::R16Float,
            Self::Rg16float => wgpu::TextureFormat::Rg16Float,
        }
    }

    pub fn bytes_per_pixel(self) -> u64 {
        match self {
            Self::Rgba32float => 16,
            Self::Rgba16float => 8,
            Self::Rgba8unorm | Self::Rg16float => 4,
            Self::R16float => 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Buffer {
    pub id: String,
    #[serde(default)]
    pub format: Format,
    /// Against the internal resolution.
    #[serde(default = "one")]
    pub scale: f32,
    /// Keeps the last frame, read as `prev:<id>`.
    #[serde(default)]
    pub persistent: bool,
    /// A mip chain is made after each write, for blurred reads.
    #[serde(default)]
    pub mips: bool,
}

fn one() -> f32 {
    1.0
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Pass {
    pub shader: String,
    /// A buffer's id, or [`OUT`].
    pub target: String,
    #[serde(default)]
    pub inputs: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(default)]
pub struct Bloom {
    pub intensity: f32,
    pub threshold: f32,
    pub radius: f32,
}

impl Default for Bloom {
    fn default() -> Self {
        Self {
            intensity: 0.6,
            threshold: 1.0,
            radius: 0.8,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tonemap {
    #[default]
    Agx,
    Aces,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(default)]
pub struct PostDefaults {
    /// In EV.
    pub exposure: f32,
    pub bloom: Bloom,
    pub tonemap: Tonemap,
    pub grain: f32,
    pub chromatic: f32,
    pub vignette: f32,
    /// The look on top of the tone mapping: 1 is neutral; AgX's "punchy" is 1.35 and 1.4.
    pub contrast: f32,
    pub saturation: f32,
}

impl Default for PostDefaults {
    fn default() -> Self {
        Self {
            exposure: 0.0,
            bloom: Bloom::default(),
            tonemap: Tonemap::default(),
            grain: 0.04,
            chromatic: 0.002,
            vignette: 0.35,
            contrast: 1.0,
            saturation: 1.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Asset {
    pub id: String,
    pub file: String,
    #[serde(default)]
    pub source: Option<String>,
    /// The pixels are data (a distance field, a height map), not sRGB colours.
    #[serde(default)]
    pub linear: bool,
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Self> {
        let manifest: Self = toml::from_str(text)?;
        manifest.validate()?;
        Ok(manifest)
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            self.format >= 1 && self.format <= FORMAT,
            "format {} is not supported (this runtime reads up to {FORMAT})",
            self.format
        );
        ensure!(!self.name.trim().is_empty(), "the scene has no name");
        ensure!(!self.license.trim().is_empty(), "the scene has no license");
        ensure!(
            self.render.scale > 0.0 && self.render.scale <= 2.0,
            "render.scale must be above 0 and at most 2"
        );
        ensure!(!self.passes.is_empty(), "the scene has no pass");
        ensure!(
            self.passes.len() <= MAX_PASSES,
            "at most {MAX_PASSES} passes"
        );
        ensure!(
            self.buffers.len() <= MAX_BUFFERS,
            "at most {MAX_BUFFERS} buffers"
        );
        ensure!(
            self.assets.len() <= MAX_ASSETS,
            "at most {MAX_ASSETS} assets"
        );

        let mut ids = HashSet::new();
        for id in self
            .params
            .iter()
            .map(|p| &p.id)
            .chain(self.buffers.iter().map(|b| &b.id))
            .chain(self.assets.iter().map(|a| &a.id))
        {
            check_identifier(id)?;
            ensure!(ids.insert(id.as_str()), "{id} is declared twice");
        }
        for param in &self.params {
            param.value()?;
            if param.kind == ParamKind::Choice {
                ensure!(
                    !param.options.is_empty(),
                    "{}: a choice needs options",
                    param.id
                );
            }
        }
        for buffer in &self.buffers {
            ensure!(
                buffer.id != OUT,
                "{OUT} is the built-in target and cannot be declared"
            );
            ensure!(
                buffer.scale > 0.0 && buffer.scale <= 2.0,
                "{}: scale must be above 0 and at most 2",
                buffer.id
            );
        }
        let buffer = |id: &str| self.buffers.iter().find(|b| b.id == id);
        let mut writes_out = false;
        for (i, pass) in self.passes.iter().enumerate() {
            let at = format!("pass {} ({})", i + 1, pass.shader);
            check_path(&pass.shader).with_context(|| at.clone())?;
            if pass.target == OUT {
                writes_out = true;
            } else {
                ensure!(
                    buffer(&pass.target).is_some(),
                    "{at}: no buffer {}",
                    pass.target
                );
            }
            for input in &pass.inputs {
                match input.strip_prefix(PREV) {
                    Some(id) => ensure!(
                        buffer(id).is_some_and(|b| b.persistent),
                        "{at}: {input} needs a persistent buffer {id}"
                    ),
                    None => {
                        ensure!(buffer(input).is_some(), "{at}: no buffer {input}");
                        ensure!(
                            *input != pass.target,
                            "{at}: reads the buffer it draws into (use {PREV}{input} for the last frame)"
                        );
                    }
                }
            }
        }
        ensure!(writes_out, "no pass draws into {OUT}");
        if let Some(state) = &self.state {
            check_path(&state.file)?;
            if let Some(update) = &state.update {
                check_path(update)?;
            }
        }
        for file in &self.common {
            check_path(file)?;
        }
        for asset in &self.assets {
            check_path(&asset.file)?;
        }
        Ok(())
    }

    /// The manifest with parameter defaults left out: when only those change, a reload keeps
    /// the pipelines and the state.
    pub fn without_defaults(&self) -> Self {
        let mut copy = self.clone();
        for param in &mut copy.params {
            param.default = toml::Value::Boolean(false);
        }
        copy
    }
}

/// Names the shaders see: ASCII letters, digits and `_`, not starting with a digit, and not with
/// the runtime's `rv_`.
fn check_identifier(id: &str) -> Result<()> {
    let mut chars = id.chars();
    let valid = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    ensure!(valid, "{id:?} is not a valid name");
    ensure!(
        !id.starts_with("rv_"),
        "{id}: names starting with rv_ are the runtime's"
    );
    Ok(())
}

/// A file inside the scene's folder.
fn check_path(path: &str) -> Result<()> {
    let ok = Path::new(path)
        .components()
        .all(|c| matches!(c, Component::Normal(_)));
    ensure!(
        ok && !path.is_empty(),
        "{path:?} is not a path inside the scene"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMALL: &str = r##"
format = 1
name = "Test"
license = "MPL-2.0"
description = { de = "Probe", en = "Test" }

[state]
file = "state.wgsl"
update = "update.wgsl"

[[param]]
id = "speed"
label = { de = "Tempo", en = "Speed" }
default = 2
min = 0
max = 8

[[param]]
id = "tint"
type = "color"
default = "#ff8000"

[[param]]
id = "look"
type = "choice"
default = 1
options = ["a", "b"]

[[buffer]]
id = "trail"
persistent = true

[[pass]]
shader = "trail.wgsl"
target = "trail"
inputs = ["prev:trail"]

[[pass]]
shader = "final.wgsl"
target = "out"
inputs = ["trail"]

[post]
bloom = { intensity = 0.9 }
tonemap = "aces"
"##;

    #[test]
    fn a_scene_reads_with_defaults_filled_in() {
        let m = Manifest::parse(SMALL).unwrap();
        assert_eq!(m.name, "Test");
        assert_eq!(m.description.as_ref().unwrap().get("de"), "Probe");
        assert_eq!(m.description.as_ref().unwrap().get("fr"), "Test");
        assert_eq!(m.params[0].value().unwrap(), Value::Float(2.0));
        assert_eq!(m.params[2].value().unwrap(), Value::Uint(1));
        let Value::Color([r, g, b]) = m.params[1].value().unwrap() else {
            panic!("not a colour");
        };
        assert!((r - 1.0).abs() < 1e-6 && (g - 0.2158).abs() < 1e-3 && b == 0.0);
        assert_eq!(m.buffers[0].format, Format::Rgba16float);
        assert_eq!(m.post.bloom.intensity, 0.9);
        assert_eq!(m.post.bloom.threshold, 1.0);
        assert_eq!(m.post.tonemap, Tonemap::Aces);
        assert_eq!(m.render.scale, 1.0);
    }

    fn broken(from: &str, to: &str) -> String {
        let text = SMALL.replacen(from, to, 1);
        assert_ne!(text, SMALL, "{from:?} is not in the scene");
        format!("{:#}", Manifest::parse(&text).unwrap_err())
    }

    #[test]
    fn mistakes_are_named() {
        assert!(broken("format = 1", "format = 2").contains("not supported"));
        assert!(broken("inputs = [\"trail\"]", "inputs = [\"glow\"]").contains("no buffer glow"));
        assert!(broken("persistent = true", "persistent = false").contains("persistent"));
        assert!(broken("inputs = [\"prev:trail\"]", "inputs = [\"trail\"]").contains("draws into"));
        assert!(
            broken(
                "target = \"out\"\ninputs = [\"trail\"]",
                "target = \"trail\""
            )
            .contains("no pass draws into out")
        );
        assert!(broken("id = \"speed\"", "id = \"rv_speed\"").contains("rv_"));
        assert!(broken("id = \"speed\"", "id = \"trail\"").contains("twice"));
        assert!(broken("shader = \"trail.wgsl\"", "shader = \"../x.wgsl\"").contains("inside"));
        assert!(broken("default = \"#ff8000\"", "default = \"orange\"").contains("#rrggbb"));
        assert!(broken("default = 1\n", "default = 5\n").contains("no option 5"));
    }

    #[test]
    fn only_defaults_differ() {
        let a = Manifest::parse(SMALL).unwrap();
        let b = Manifest::parse(&SMALL.replace("default = 2", "default = 3")).unwrap();
        assert_ne!(a, b);
        assert_eq!(a.without_defaults(), b.without_defaults());
    }
}
