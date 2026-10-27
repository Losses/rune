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
    /// Whether the scene reads the `covers` texture: an atlas of album covers from the library.
    #[serde(default)]
    pub covers: bool,
    /// How the camera director may tilt and cut the scene's view (see `director`).
    #[serde(default)]
    pub camera: Camera,
    /// The camera setups the scene can be shown from. Without any, the scene has no director.
    #[serde(default, rename = "shot")]
    pub shots: Vec<Shot>,
    /// The transitions and lens effects the scene takes (see [`Effects`]).
    #[serde(default)]
    pub effects: Effects,
}

/// The `[camera]` table: what the director may do with the scene's shots.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Default)]
#[serde(default)]
pub struct Camera {
    /// The largest dutch angle (roll) in radians; 0 means the scene never tilts.
    pub dutch: f32,
    /// Whether the director may cut on every beat before a section or a phrase ends.
    pub fill: bool,
    /// The shortest shot in bars (0 for no minimum): a scene that glides to a new shot sets it
    /// to how long its glide takes, so the shot is reached before the next cut. Bursts ignore it.
    pub hold: u32,
    /// How often the director cuts the scene: what suits its subject.
    pub pace: Pace,
}

/// How often the director cuts a scene, against the section's energy (see `director`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Pace {
    /// Only at a section's start (and in the burst before a lift).
    Sections,
    /// Half as often as `normal`.
    Calm,
    #[default]
    Normal,
    /// Twice as often as `normal`, with a burst at the end of every phrase of a high section.
    Busy,
}

impl Pace {
    /// The word `scene.toml` uses for it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Sections => "sections",
            Self::Calm => "calm",
            Self::Normal => "normal",
            Self::Busy => "busy",
        }
    }
}

/// The transitions that are known by name: `cut` and those the runtime draws (see
/// `crate::transition`), and those still to come, which a scene may already list.
pub const TRANSITIONS: &[&str] = &[
    "cut", "fade", "dissolve", "cutin", "flash", "glitch", "mosaic",
];
/// The lens effects that are known by name: those the runtime draws (see `scene::lens`), and
/// `blur`, still to come. (`blocks`, a colour-block frame, is now the start of a `flash`.)
pub const LENS_EFFECTS: &[&str] = &["flare", "leak", "blur"];
/// The transitions that also play as a lens effect over the picture, at a strong moment with no
/// cut, when the scene lists them (see `crate::moments::lens_effect`).
pub const LENS_TRANSITIONS: &[&str] = &["glitch", "mosaic"];

/// The `effects` list: the transitions and lens effects that suit the scene, one list for a switch
/// to the scene and for the director's cuts within it. The scene says what may be used; the runtime
/// chooses when, from the music's moments (see `crate::moments`). Kept apart by kind here, for the
/// parts of the runtime that use them. A scene without the list gets the plain ones.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(from = "Vec<String>")]
pub struct Effects {
    /// The names as the scene lists them.
    pub names: Vec<String>,
    /// The transitions a switch to this scene may take (the scene coming decides, not the one
    /// going). A fade or a dissolve too, unless the list names a plain transition itself.
    pub switch: Vec<String>,
    /// The transitions that may dress a cut of the director within the scene: those of the list
    /// that play over a cut (not `cut` itself, and not `cutin`, which needs a new scene).
    pub cut: Vec<String>,
    /// The lens effects the scene takes: those of the list, and its glitch and mosaic, which also
    /// play over the picture.
    pub lens: Vec<String>,
}

impl Default for Effects {
    fn default() -> Self {
        Self {
            names: Vec::new(),
            switch: vec!["fade".into(), "dissolve".into()],
            cut: Vec::new(),
            lens: Vec::new(),
        }
    }
}

impl From<Vec<String>> for Effects {
    fn from(names: Vec<String>) -> Self {
        let of = |known: &[&str]| -> Vec<String> {
            names
                .iter()
                .filter(|name| known.contains(&name.as_str()))
                .cloned()
                .collect()
        };
        // A list that names none of the plain transitions keeps them for the plain switches.
        let mut switch = of(TRANSITIONS);
        if !switch
            .iter()
            .any(|n| ["cut", "fade", "dissolve"].contains(&n.as_str()))
        {
            switch.splice(0..0, ["fade".to_string(), "dissolve".to_string()]);
        }
        Self {
            switch,
            cut: of(&["fade", "dissolve", "flash", "glitch", "mosaic"]),
            lens: names
                .iter()
                .filter(|n| {
                    LENS_EFFECTS.contains(&n.as_str()) || LENS_TRANSITIONS.contains(&n.as_str())
                })
                .cloned()
                .collect(),
            names,
        }
    }
}

impl Effects {
    /// The names the runtime does not know (an older runtime than the scene's author used, or a
    /// typo): they are left out with a warning, not refused.
    pub fn unknown(&self) -> Vec<String> {
        self.names
            .iter()
            .filter(|name| !TRANSITIONS.contains(&name.as_str()))
            .filter(|name| !LENS_EFFECTS.contains(&name.as_str()))
            .map(|name| format!("effects = {name:?}"))
            .collect()
    }
}

/// How near a shot looks at its subject. The order is the order of the numbers `struct Shot`
/// gives the scene as `size`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShotSize {
    Wide,
    Medium,
    Close,
    Detail,
    Top,
}

/// How a scene gets from one shot to the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShotMove {
    /// The view jumps (the transition layer may dress it).
    #[default]
    Cut,
    /// The scene slides to the new view itself.
    Glide,
}

/// One `[[shot]]`: a camera setup of the scene.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Shot {
    /// For people.
    #[serde(default)]
    pub name: String,
    pub size: ShotSize,
    #[serde(default, rename = "move")]
    pub motion: ShotMove,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Render {
    /// The internal resolution against the output's: the most it is; the runtime lowers it to
    /// half of this when frames take too long on the GPU (see `crate::adaptive`).
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

/// A colour given as 0xAARRGGBB (sRGB), linear; the alpha is dropped.
pub fn argb_to_linear(argb: u32) -> [f32; 3] {
    [16, 8, 0].map(|shift| srgb_to_linear(((argb >> shift) & 0xff) as f32 / 255.0))
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
        ensure!(
            self.camera.dutch.is_finite()
                && (0.0..=std::f32::consts::FRAC_PI_2).contains(&self.camera.dutch),
            "camera.dutch must be from 0 to {:.4} radians",
            std::f32::consts::FRAC_PI_2
        );
        for asset in &self.assets {
            check_path(&asset.file)?;
        }
        ensure!(
            self.camera == Camera::default() || !self.shots.is_empty(),
            "[camera] needs at least one [[shot]]"
        );
        for unknown in self.effects.unknown() {
            log::warn!("{unknown} is not an effect this runtime knows; it is left out");
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

    #[test]
    fn covers_are_off_unless_declared() {
        assert!(!Manifest::parse(SMALL).unwrap().covers);
        let with = SMALL.replacen(
            "license = \"MPL-2.0\"\n",
            "license = \"MPL-2.0\"\ncovers = true\n",
            1,
        );
        assert!(Manifest::parse(&with).unwrap().covers);
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

    const SHOTS: &str = r##"
[camera]
dutch = 0.35
fill = true
hold = 4
pace = "calm"

[[shot]]
name = "below"
size = "medium"
move = "cut"

[[shot]]
size = "top"
move = "glide"
"##;

    #[test]
    fn shots_and_the_camera_are_read() {
        let plain = Manifest::parse(SMALL).unwrap();
        assert!(plain.shots.is_empty());
        assert_eq!(plain.camera, Camera::default());
        assert!(!plain.camera.fill && plain.camera.dutch == 0.0 && plain.camera.hold == 0);

        let m = Manifest::parse(&format!("{SMALL}{SHOTS}")).unwrap();
        assert_eq!(m.camera.dutch, 0.35);
        assert!(m.camera.fill);
        assert_eq!(m.camera.hold, 4);
        assert_eq!(m.camera.pace, Pace::Calm);
        assert_eq!(plain.camera.pace, Pace::Normal);
        assert_eq!(m.shots.len(), 2);
        assert_eq!(m.shots[0].name, "below");
        assert_eq!(m.shots[0].size, ShotSize::Medium);
        assert_eq!(m.shots[0].motion, ShotMove::Cut);
        assert_eq!(m.shots[1].name, "");
        assert_eq!(m.shots[1].size, ShotSize::Top);
        assert_eq!(m.shots[1].motion, ShotMove::Glide);
    }

    #[test]
    fn a_bad_shot_is_named() {
        let with = |from: &str, to: &str| {
            let text = format!("{SMALL}{SHOTS}").replacen(from, to, 1);
            assert_ne!(
                text,
                format!("{SMALL}{SHOTS}"),
                "{from:?} is not in the shots"
            );
            format!("{:#}", Manifest::parse(&text).unwrap_err())
        };
        let size = with("size = \"medium\"", "size = \"huge\"");
        assert!(size.contains("huge") && size.contains("wide"), "{size}");
        let motion = with("move = \"glide\"", "move = \"fly\"");
        assert!(motion.contains("fly") && motion.contains("cut"), "{motion}");
        assert!(with("size = \"top\"", "").contains("size"));
        assert!(with("dutch = 0.35", "dutch = 4.0").contains("camera.dutch"));
        let none = format!("{SMALL}\n[camera]\nfill = true\n");
        let none = format!("{:#}", Manifest::parse(&none).unwrap_err());
        assert!(none.contains("at least one [[shot]]"), "{none}");
        let pace = with("pace = \"calm\"", "pace = \"frantic\"");
        assert!(
            pace.contains("frantic") && pace.contains("sections"),
            "{pace}"
        );
    }

    #[test]
    fn effects_default_to_the_plain_ones_and_unknown_names_are_only_named() {
        let plain = Manifest::parse(SMALL).unwrap();
        assert_eq!(plain.effects.switch, ["fade", "dissolve"]);
        assert!(plain.effects.cut.is_empty() && plain.effects.lens.is_empty());
        assert!(plain.effects.unknown().is_empty());

        // One list, kept apart by kind. It is a top-level key, so it goes before the tables.
        let listed = SMALL.replacen(
            "format = 1\n",
            "format = 1\neffects = [\"fade\", \"flash\", \"cutin\", \"flare\", \"sparkle\"]\n",
            1,
        );
        let m = Manifest::parse(&listed).unwrap();
        assert_eq!(m.effects.switch, ["fade", "flash", "cutin"]);
        assert_eq!(m.effects.cut, ["fade", "flash"]);
        assert_eq!(m.effects.lens, ["flare"]);
        assert_eq!(m.effects.unknown(), ["effects = \"sparkle\""]);
        // A list without a plain transition keeps the fade and the dissolve for plain switches.
        let strong = SMALL.replacen("format = 1\n", "format = 1\neffects = [\"glitch\"]\n", 1);
        let strong = Manifest::parse(&strong).unwrap().effects;
        assert_eq!(strong.switch, ["fade", "dissolve", "glitch"]);
        // A glitch or a mosaic is a lens effect too.
        assert_eq!(strong.lens, ["glitch"]);
    }
}
