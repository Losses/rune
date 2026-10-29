//! `scene.toml`: what a scene is made of. See `docs/scenes.md` for every key.

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
pub const MAX_INPUTS: usize = 32;
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
    /// The values the runtime drives from the music, read in the shaders as `input.<id>`.
    #[serde(default, rename = "input")]
    pub inputs: Vec<InputDecl>,
    /// What drives each input: a source of the music, how, and what to do where it means
    /// nothing (see `crate::bind`).
    #[serde(default, rename = "bind")]
    pub binds: Vec<Bind>,
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
    /// The shots the director's burst of cuts goes through, in the order it cuts to them (see
    /// [`BurstShot`]). Without any, the scene has no burst.
    #[serde(default, rename = "burst")]
    pub bursts: Vec<BurstShot>,
    /// The looks of the scene: large visual changes (the light, the palette) the runtime changes
    /// between by the music (see [`Look`]). Without any, the scene has none.
    #[serde(default, rename = "look")]
    pub looks: Vec<Look>,
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
    /// The shortest shot in seconds (0 for no minimum): a scene that glides to a new shot sets
    /// it to how long its glide takes, so the shot is reached before the next cut. It only ever
    /// raises the director's gap between two cuts for this scene (`director.cadence`'s `gap`).
    /// Bursts ignore it.
    pub hold: f32,
    /// How often the director cuts the scene: what suits its subject.
    pub pace: Pace,
    /// Which cuts tilt by the dutch angle: all of them but wide and top ones, or only a burst's (a
    /// tilt on every cut was too much for Neon Street).
    #[serde(rename = "dutch_in")]
    pub dutch_in: DutchIn,
}

/// Which cuts the dutch angle tilts (`[camera] dutch_in`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DutchIn {
    /// Every cut to a medium, close or detail shot.
    #[default]
    All,
    /// Only the cuts of a burst.
    Bursts,
}

/// How often the director cuts a scene, against the section's energy (see `director`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Pace {
    /// Only at a section's start (and in the burst before a lift).
    Sections,
    /// One and a half times the seconds of `normal` between two cuts.
    Calm,
    #[default]
    Normal,
    /// One and a half times as often as `normal`, with a burst at the end of every phrase of a high section.
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
/// `blur`, still to come. (`blocks`, a colour-block frame, is now the start of a `flash`.) The
/// `glitch` and the `mosaic` exist as both a transition and a lens effect.
pub const LENS_EFFECTS: &[&str] = &["flare", "leak", "blur", "glitch", "mosaic"];

/// The `[effects]` table: the transitions and the lens effects that suit the scene, declared apart.
/// A transition happens at a change of picture: switching into the scene, and a hard cut of the
/// director's camera in it (one list for both). A lens effect lies over a picture that goes on. The
/// glitch and the mosaic are both, so a scene says for which. The scene says what may be used; the
/// runtime chooses when, from the music's moments (see `crate::moments`). A scene without the table
/// gets the plain transitions and no lens effect.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(try_from = "RawEffects")]
pub struct Effects {
    /// The transitions a switch to this scene may take (the scene coming decides, not the one
    /// going). A fade or a dissolve too, unless `transitions` names a plain one itself.
    pub switch: Vec<String>,
    /// The transitions that may dress a cut of the director within the scene: those of
    /// `transitions` that play over a cut (not `cut` itself, and not `cutin`, which needs a new
    /// scene).
    pub cut: Vec<String>,
    /// The lens effects the scene takes: exactly those of `lens`.
    pub lens: Vec<String>,
    /// The names of either list the runtime does not know, as `transitions = "x"`.
    unknown: Vec<String>,
}

/// `[effects]` as the file writes it. Any other key is refused: a top-level key written after the
/// table would otherwise land in it unnoticed.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawEffects {
    #[serde(default)]
    transitions: Vec<String>,
    #[serde(default)]
    lens: Vec<String>,
}

impl Default for Effects {
    fn default() -> Self {
        Self {
            switch: vec!["fade".into(), "dissolve".into()],
            cut: Vec::new(),
            lens: Vec::new(),
            unknown: Vec::new(),
        }
    }
}

impl TryFrom<RawEffects> for Effects {
    type Error = String;

    fn try_from(raw: RawEffects) -> Result<Self, String> {
        let mut unknown = Vec::new();
        for name in &raw.transitions {
            if TRANSITIONS.contains(&name.as_str()) {
            } else if LENS_EFFECTS.contains(&name.as_str()) {
                return Err(format!(
                    "[effects] transitions: {name:?} is a lens effect, which lies over the picture: put it in lens = [...]"
                ));
            } else {
                unknown.push(format!("[effects] transitions = {name:?}"));
            }
        }
        for name in &raw.lens {
            if LENS_EFFECTS.contains(&name.as_str()) {
            } else if TRANSITIONS.contains(&name.as_str()) {
                return Err(format!(
                    "[effects] lens: {name:?} is a transition, which happens at a change of picture: put it in transitions = [...]"
                ));
            } else {
                unknown.push(format!("[effects] lens = {name:?}"));
            }
        }
        let of = |known: &[&str]| -> Vec<String> {
            raw.transitions
                .iter()
                .filter(|name| known.contains(&name.as_str()))
                .cloned()
                .collect()
        };
        // Transitions that name none of the plain ones keep them for the plain switches.
        let mut switch = of(TRANSITIONS);
        if !switch
            .iter()
            .any(|n| ["cut", "fade", "dissolve"].contains(&n.as_str()))
        {
            switch.splice(0..0, ["fade".to_string(), "dissolve".to_string()]);
        }
        Ok(Self {
            switch,
            cut: of(&["fade", "dissolve", "flash", "glitch", "mosaic"]),
            lens: raw
                .lens
                .iter()
                .filter(|name| LENS_EFFECTS.contains(&name.as_str()))
                .cloned()
                .collect(),
            unknown,
        })
    }
}

impl Effects {
    /// The names the runtime does not know (an older runtime than the scene's author used, or a
    /// typo): they are left out with a warning, not refused.
    pub fn unknown(&self) -> Vec<String> {
        self.unknown.clone()
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
    /// Whether the shot may open a section. Left out, a wide shot may and the others may not
    /// (see [`Shot::opens`]).
    #[serde(default)]
    pub opens: Option<bool>,
    /// How often the director picks this shot against the scene's others: 0 never (while another
    /// has a chance), 2 twice as often as 1. It is finite and 0 or more.
    #[serde(default = "one")]
    pub weight: f32,
    /// The longest the shot may last, in seconds: after that the director cuts away at the next
    /// place a cut lands on, however long the cadence's wait (the gap still holds). For a shot that
    /// tires the eye if it stays (Torii Sea's near ones). None: as long as the cadence keeps it.
    #[serde(default)]
    pub longest: Option<f32>,
    /// The kinds of section, by energy, the shot suits: any of "quiet", "normal", "high"; empty for
    /// all. The director picks among the shots that suit the section (Neon Street's far views are
    /// not for its climax), and among all if none does.
    #[serde(default)]
    pub tiers: Vec<String>,
    /// Whether the shot tilts by the dutch angle, whatever `[camera] dutch_in` says: true tilts it
    /// even in a scene that tilts only its bursts (Neon Street's street side), false keeps it
    /// level. None: as `dutch_in` says. A wide or a top shot never tilts, so it may not say true.
    #[serde(default)]
    pub dutch: Option<bool>,
    /// Whether the director cuts to this shot at least once in a song that has a chart: picked by
    /// chance as any other, and if it has not been on by the last section that suits it, cut to
    /// there (Paperclip's circling shot, which only high sections suit, could be left out of a
    /// whole song by the weights). False: only chance decides.
    #[serde(default)]
    pub at_least_once: bool,
}

impl Shot {
    /// Whether the shot may open a section: what `opens` says, else only a wide shot does, since a
    /// section is established by looking at all of it.
    pub fn opens(&self) -> bool {
        self.opens.unwrap_or(self.size == ShotSize::Wide)
    }
}

/// One `[[look]]`: a large visual change of the scene (its light, its palette), which the scene
/// draws and the runtime decides when to change to (`docs/music.md`, "Looks"). A change of what
/// moves or what is drawn (a formation, a dance) is not a look.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Look {
    /// For people: the lab lists it.
    #[serde(default)]
    pub name: String,
    /// Hints for the runtime and the lab ("warm", "night"). Only declared for now: nothing reads
    /// them yet.
    #[serde(default)]
    pub tags: Vec<String>,
    /// How often the runtime picks this look against the scene's others for a kind of section: 0
    /// never (while another has a chance), 2 twice as often as 1. It is finite and 0 or more.
    #[serde(default = "one")]
    pub weight: f32,
    /// Whether the scene may start in this look. A scene that never starts in daylight sets it to
    /// false on its day.
    #[serde(default = "yes")]
    pub opens: bool,
    /// The kinds of section, by energy, the look suits: any of "quiet", "normal", "high"; empty
    /// for all. A section start picks among the looks that suit it (a scene grey in its quiet
    /// sections and in the theme colour at its climax), and among all if none does.
    #[serde(default)]
    pub tiers: Vec<String>,
    /// Whether the look is on at least once in a song that has a chart: picked by chance as any
    /// other, and if it has not been on by the last section start that suits it, changed to there
    /// (Torii Sea's day). False: only chance decides.
    #[serde(default)]
    pub at_least_once: bool,
}

impl Look {
    /// Whether the look suits a section of tier `tier` (0 quiet, 1 normal, 2 high).
    pub fn suits(&self, tier: usize) -> bool {
        suits(&self.tiers, tier)
    }
}

/// The names of the tiers a look or a shot may say it suits, quietest first.
pub const TIERS: [&str; 3] = ["quiet", "normal", "high"];

/// Whether `tiers` (empty for all) holds tier `tier` (0 quiet, 1 normal, 2 high).
pub fn suits(tiers: &[String], tier: usize) -> bool {
    tiers.is_empty() || tiers.iter().any(|t| t == TIERS[tier.min(2)])
}

/// The most `[[burst]]` shots a scene declares: the burst cuts on one beat each, and four is the
/// shape of the burst.
pub const MAX_BURSTS: usize = 4;

/// One `[[burst]]`: a shot the burst of cuts before a section that lifts the energy goes to. It
/// has the fields of a `[[shot]]` but its move, which is always a hard cut, and it is never
/// picked by an ordinary cut: a burst is four hard cuts in two seconds, the hardest thing the
/// director does to get right, so a scene declares its shots on their own (`docs/music.md`,
/// "Shots and the director").
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BurstShot {
    /// For people.
    #[serde(default)]
    pub name: String,
    pub size: ShotSize,
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

/// One `[[input]]`: a value of the scene's that the runtime drives from the music through a
/// `[[bind]]`; the shaders read it as `input.<id>`, always one `f32`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputDecl {
    pub id: String,
    /// For people.
    #[serde(default)]
    pub label: Option<Text>,
    /// The range of values the input takes: what a `follow` maps to, and where a `trigger` peaks
    /// (at the top).
    #[serde(default = "unit_range")]
    pub range: [f32; 2],
    /// The value with nothing driving it; the bottom of the range if left out.
    #[serde(default)]
    pub rest: Option<f32>,
}

fn unit_range() -> [f32; 2] {
    [0.0, 1.0]
}

impl InputDecl {
    /// The input as TOML, the keys of its `[[input]]` table (for a lab that writes it back).
    pub fn declared(&self) -> String {
        use crate::bind::declared::{inline, short};
        use toml::Value;
        let mut text = format!("id = {}\n", inline(&Value::String(self.id.clone())));
        match &self.label {
            None => {}
            Some(Text::Plain(label)) => {
                text += &format!("label = {}\n", inline(&Value::String(label.clone())));
            }
            Some(Text::Localized(map)) => {
                let map = map
                    .iter()
                    .map(|(lang, label)| (lang.clone(), Value::String(label.clone())))
                    .collect();
                text += &format!("label = {}\n", inline(&Value::Table(map)));
            }
        }
        let range = Value::Array(self.range.map(|end| Value::Float(short(end))).to_vec());
        text += &format!("range = {}\n", inline(&range));
        if let Some(rest) = self.rest {
            text += &format!("rest = {}\n", inline(&Value::Float(short(rest))));
        }
        text
    }

    /// The value with nothing driving it.
    pub fn rest(&self) -> f32 {
        self.rest.unwrap_or(self.range[0])
    }
}

/// What a `[[bind]]` does where its source means nothing: the name of another source, or the
/// word `hold` or `rest`, or a table `{ value = x }`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum FallbackDecl {
    Word(String),
    Value { value: f32 },
}

/// One `[[bind]]`: a source of the music drives an input in some way (see `crate::bind`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bind {
    /// The `id` of the input it drives.
    pub target: String,
    /// A name of the registry of sources (`crate::bind::SOURCES`).
    pub source: String,
    /// `follow`, `trigger`, `step`, `pick` or `accumulate`.
    pub how: String,
    /// The parameters of the `how`.
    #[serde(default)]
    pub when: toml::Table,
    /// `out` eases a beat time: a push at the start of every beat (`follow` and `step`).
    #[serde(default)]
    pub ease: Option<String>,
    #[serde(default)]
    pub fallback: Option<FallbackDecl>,
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

fn yes() -> bool {
    true
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

/// The `[[input]]` and `[[bind]]` arrays of a scene on their own (see
/// [`Manifest::with_bindings`]).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BindingsText {
    #[serde(default, rename = "input")]
    inputs: Vec<InputDecl>,
    #[serde(default, rename = "bind")]
    binds: Vec<Bind>,
}

impl Manifest {
    /// This manifest with the inputs and binds of `text`, TOML that holds those two arrays and
    /// nothing else. Checked as a whole manifest is, so that a mistake is refused with the same
    /// message the file would give.
    pub fn with_bindings(&self, text: &str) -> Result<Self> {
        let parsed: BindingsText = toml::from_str(text).context("the scene's bindings")?;
        let mut next = self.clone();
        next.inputs = parsed.inputs;
        next.binds = parsed.binds;
        next.validate()?;
        Ok(next)
    }

    pub fn parse(text: &str) -> Result<Self> {
        // The keys were removed, and a table without them would read the file the same way (or fail
        // with a word about types), so they are looked for by name.
        let table = text.parse::<toml::Table>()?;
        ensure!(
            !table.get("effects").is_some_and(toml::Value::is_array),
            "the top-level effects = [...] list was removed: declare an [effects] table instead, \
             transitions = [...] for the switch into the scene and the cuts of its camera, \
             lens = [...] for the effects over the picture"
        );
        let manifest: Self = toml::from_str(text)?;
        let removed = table
            .get("camera")
            .and_then(|camera| camera.get("fill"))
            .is_some();
        ensure!(
            !removed,
            "[camera] fill was removed: declare the burst's shots as [[burst]] entries instead"
        );
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
            .chain(self.inputs.iter().map(|i| &i.id))
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
        ensure!(
            self.inputs.len() <= MAX_INPUTS,
            "at most {MAX_INPUTS} inputs"
        );
        for input in &self.inputs {
            let [lo, hi] = input.range;
            ensure!(
                lo.is_finite() && hi.is_finite() && lo < hi,
                "{}: range must be [lo, hi] with lo below hi",
                input.id
            );
            ensure!(
                input.rest.is_none_or(f32::is_finite),
                "{}: rest must be a finite number",
                input.id
            );
        }
        let mut driven = HashSet::new();
        for bind in &self.binds {
            let input = self
                .inputs
                .iter()
                .find(|i| i.id == bind.target)
                .with_context(|| {
                    format!(
                        "[[bind]] for `{}`: no input of that name (declare it with [[input]])",
                        bind.target
                    )
                })?;
            ensure!(
                driven.insert(bind.target.as_str()),
                "[[bind]] for `{}`: the input has more than one bind",
                bind.target
            );
            crate::bind::Evaluator::new_in(
                bind,
                input,
                crate::bind::Scope {
                    params: &self.params,
                    inputs: &self.inputs,
                },
            )?;
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
            self.camera.hold.is_finite() && self.camera.hold >= 0.0,
            "camera.hold must be a number of seconds, 0 or more"
        );
        ensure!(
            self.camera == Camera::default() || !self.shots.is_empty(),
            "[camera] needs at least one [[shot]]"
        );
        for (i, shot) in self.shots.iter().enumerate() {
            ensure!(
                shot.weight.is_finite() && shot.weight >= 0.0,
                "[[shot]] {} ({:?}): weight must be a number, 0 or more, not {}",
                i + 1,
                shot.name,
                shot.weight
            );
            ensure!(
                shot.longest.is_none_or(|s| s.is_finite() && s > 0.0),
                "[[shot]] {} ({:?}): longest must be a number of seconds above 0",
                i + 1,
                shot.name
            );
            ensure!(
                shot.dutch != Some(true) || !matches!(shot.size, ShotSize::Wide | ShotSize::Top),
                "[[shot]] {} ({:?}): a wide or top shot never tilts, so it may not say dutch = true",
                i + 1,
                shot.name
            );
            for tier in &shot.tiers {
                ensure!(
                    TIERS.contains(&tier.as_str()),
                    "[[shot]] {} ({:?}): tiers takes \"quiet\", \"normal\" and \"high\", not {:?}",
                    i + 1,
                    shot.name,
                    tier
                );
            }
        }
        ensure!(
            self.shots.is_empty() || self.shots.iter().any(|shot| shot.weight > 0.0),
            "the [[shot]]s all have weight = 0, so the director could never pick one: give at least one a weight above 0"
        );
        ensure!(
            self.bursts.len() <= MAX_BURSTS,
            "at most {MAX_BURSTS} [[burst]] shots (one cut on each of the last {MAX_BURSTS} beats), not {}",
            self.bursts.len()
        );
        ensure!(
            self.bursts.is_empty() || !self.shots.is_empty(),
            "[[burst]] needs at least one [[shot]]"
        );
        for (i, look) in self.looks.iter().enumerate() {
            ensure!(
                look.weight.is_finite() && look.weight >= 0.0,
                "[[look]] {} ({:?}): weight must be a number, 0 or more, not {}",
                i + 1,
                look.name,
                look.weight
            );
        }
        ensure!(
            self.looks.is_empty() || self.looks.iter().any(|look| look.weight > 0.0),
            "the [[look]]s all have weight = 0, so the runtime could never pick one: give at least one a weight above 0"
        );
        ensure!(
            self.looks.is_empty() || self.looks.iter().any(|look| look.opens),
            "the [[look]]s all have opens = false, so the scene could not start in any: let at least one open"
        );
        for (i, look) in self.looks.iter().enumerate() {
            for tier in &look.tiers {
                ensure!(
                    matches!(tier.as_str(), "quiet" | "normal" | "high"),
                    "[[look]] {} ({:?}): tiers takes \"quiet\", \"normal\" and \"high\", not {:?}",
                    i + 1,
                    look.name,
                    tier
                );
            }
        }
        for unknown in self.effects.unknown() {
            log::warn!("{unknown} is not an effect this runtime knows; it is left out");
        }
        Ok(())
    }

    /// The manifest with parameter defaults and the binds left out: when only those change, a
    /// reload keeps the pipelines and the state (the binds are only evaluated on the CPU; the
    /// inputs they drive are the struct's layout and stay).
    pub fn without_defaults(&self) -> Self {
        let mut copy = self.clone();
        copy.binds.clear();
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
hold = 8.0
pace = "calm"

[[shot]]
name = "below"
size = "medium"
move = "cut"

[[shot]]
size = "top"
move = "glide"

[[burst]]
name = "in"
size = "close"

[[burst]]
size = "detail"
"##;

    #[test]
    fn shots_and_the_camera_are_read() {
        let plain = Manifest::parse(SMALL).unwrap();
        assert!(plain.shots.is_empty());
        assert_eq!(plain.camera, Camera::default());
        assert!(plain.camera.dutch == 0.0 && plain.camera.hold == 0.0);
        assert!(plain.bursts.is_empty());

        let m = Manifest::parse(&format!("{SMALL}{SHOTS}")).unwrap();
        assert_eq!(m.camera.dutch, 0.35);
        assert_eq!(m.camera.hold, 8.0);
        assert_eq!(m.camera.pace, Pace::Calm);
        assert_eq!(plain.camera.pace, Pace::Normal);
        assert_eq!(m.shots.len(), 2);
        assert_eq!(m.shots[0].name, "below");
        assert_eq!(m.shots[0].size, ShotSize::Medium);
        assert_eq!(m.shots[0].motion, ShotMove::Cut);
        assert_eq!(m.shots[1].name, "");
        assert_eq!(m.shots[1].size, ShotSize::Top);
        assert_eq!(m.shots[1].motion, ShotMove::Glide);
        assert_eq!(m.bursts.len(), 2);
        assert_eq!(m.bursts[0].name, "in");
        assert_eq!(m.bursts[0].size, ShotSize::Close);
        assert_eq!(m.bursts[1].name, "");
        assert_eq!(m.bursts[1].size, ShotSize::Detail);
    }

    #[test]
    fn a_shot_opens_a_section_only_if_wide_or_it_says_so() {
        let m = Manifest::parse(&format!("{SMALL}{SHOTS}")).unwrap();
        // Medium and top by default do not; both have the default weight.
        assert!(!m.shots[0].opens() && !m.shots[1].opens());
        assert_eq!((m.shots[0].weight, m.shots[1].weight), (1.0, 1.0));
        let text = format!("{SMALL}{SHOTS}")
            .replacen(
                "size = \"top\"",
                "size = \"top\"\nopens = true\nweight = 2.5",
                1,
            )
            .replacen("size = \"medium\"", "size = \"wide\"", 1);
        let m = Manifest::parse(&text).unwrap();
        assert!(m.shots[0].opens() && m.shots[1].opens());
        assert_eq!(m.shots[1].weight, 2.5);
        let closed = format!("{SMALL}{SHOTS}").replacen(
            "size = \"medium\"",
            "size = \"wide\"\nopens = false",
            1,
        );
        assert!(!Manifest::parse(&closed).unwrap().shots[0].opens());
    }

    #[test]
    fn a_bad_weight_is_named_and_all_zero_is_refused() {
        let with = |from: &str, to: &str| {
            let text = format!("{SMALL}{SHOTS}").replacen(from, to, 1);
            assert_ne!(
                text,
                format!("{SMALL}{SHOTS}"),
                "{from:?} is not in the shots"
            );
            format!("{:#}", Manifest::parse(&text).unwrap_err())
        };
        let negative = with("size = \"top\"", "size = \"top\"\nweight = -1");
        assert!(
            negative.contains("weight") && negative.contains("0 or more"),
            "{negative}"
        );
        let nan = with("size = \"top\"", "size = \"top\"\nweight = nan");
        assert!(nan.contains("weight"), "{nan}");
        let longest = with("size = \"top\"", "size = \"top\"\nlongest = 0");
        assert!(longest.contains("longest"), "{longest}");
        let tilted_top = with("size = \"top\"", "size = \"top\"\ndutch = true");
        assert!(tilted_top.contains("never tilts"), "{tilted_top}");
        let text = format!("{SMALL}{SHOTS}").replacen(
            "size = \"top\"",
            "size = \"top\"\ndutch = false",
            1,
        );
        let m = Manifest::parse(&text).unwrap();
        assert!(m.shots.iter().any(|shot| shot.dutch == Some(false)));
        let text = format!("{SMALL}{SHOTS}").replacen(
            "size = \"top\"",
            "size = \"top\"\nlongest = 6.5",
            1,
        );
        let m = Manifest::parse(&text).unwrap();
        assert!(m.shots.iter().any(|shot| shot.longest == Some(6.5)));
        // A shot the song is promised is a flag, off where left out.
        let text = format!("{SMALL}{SHOTS}").replacen(
            "size = \"top\"",
            "size = \"top\"\nat_least_once = true",
            1,
        );
        let m = Manifest::parse(&text).unwrap();
        assert_eq!(m.shots.iter().filter(|shot| shot.at_least_once).count(), 1);
        // One shot at 0 is fine, all of them are not.
        let text =
            format!("{SMALL}{SHOTS}").replacen("size = \"top\"", "size = \"top\"\nweight = 0", 1);
        assert!(Manifest::parse(&text).is_ok());
        let all = text.replacen("size = \"medium\"", "size = \"medium\"\nweight = 0", 1);
        let all = format!("{:#}", Manifest::parse(&all).unwrap_err());
        assert!(
            all.contains("weight = 0") && all.contains("[[shot]]"),
            "{all}"
        );
    }

    const LOOKS: &str = r##"
[[look]]
name = "low sun"
tags = ["warm"]

[[look]]
name = "day"
weight = 2.5
opens = false

[[look]]
"##;

    #[test]
    fn looks_are_read_with_their_defaults() {
        assert!(Manifest::parse(SMALL).unwrap().looks.is_empty());
        let m = Manifest::parse(&format!("{SMALL}{LOOKS}")).unwrap();
        assert_eq!(m.looks.len(), 3);
        assert_eq!(m.looks[0].name, "low sun");
        assert_eq!(m.looks[0].tags, ["warm"]);
        assert_eq!((m.looks[0].weight, m.looks[0].opens), (1.0, true));
        assert_eq!((m.looks[1].weight, m.looks[1].opens), (2.5, false));
        assert_eq!(m.looks[2].name, "");
        assert!(m.looks.iter().all(|look| !look.at_least_once));
        assert!(m.looks[2].tags.is_empty());
    }

    #[test]
    fn a_look_says_the_sections_it_suits_and_a_camera_may_tilt_in_bursts_only() {
        let text = format!(
            "{SMALL}\n[camera]\ndutch = 0.3\ndutch_in = \"bursts\"\n\n[[shot]]\nsize = \"wide\"\n\n[[look]]\nname = \"grey\"\ntiers = [\"quiet\", \"normal\"]\n\n[[look]]\nname = \"colour\"\ntiers = [\"high\"]\n"
        );
        let m = Manifest::parse(&text).unwrap();
        assert_eq!(m.camera.dutch_in, DutchIn::Bursts);
        assert!(m.looks[0].suits(0) && m.looks[0].suits(1) && !m.looks[0].suits(2));
        assert!(m.looks[1].suits(2) && !m.looks[1].suits(0));
        let bad = text.replace("\"high\"]", "\"loud\"]");
        let error = format!("{:#}", Manifest::parse(&bad).unwrap_err());
        assert!(error.contains("tiers") && error.contains("loud"), "{error}");
    }

    #[test]
    fn a_bad_look_is_named() {
        let with = |from: &str, to: &str| {
            let text = format!("{SMALL}{LOOKS}").replacen(from, to, 1);
            assert_ne!(
                text,
                format!("{SMALL}{LOOKS}"),
                "{from:?} is not in the looks"
            );
            format!("{:#}", Manifest::parse(&text).unwrap_err())
        };
        let negative = with("weight = 2.5", "weight = -1");
        assert!(
            negative.contains("[[look]] 2") && negative.contains("0 or more"),
            "{negative}"
        );
        assert!(with("weight = 2.5", "weight = nan").contains("weight"));
        let key = with("opens = false", "open = false");
        assert!(key.contains("open") && key.contains("opens"), "{key}");
        assert!(with("tags = [\"warm\"]", "tags = \"warm\"").contains("tags"));
        let parse = |looks: &str| Manifest::parse(&format!("{SMALL}{looks}"));
        let closed = "[[look]]\nopens = false\n[[look]]\nopens = false\n";
        let message = format!("{:#}", parse(closed).unwrap_err());
        assert!(message.contains("opens = false"), "{message}");
        // One look at weight 0 is fine, all of them are not.
        assert!(parse("[[look]]\nweight = 0\n[[look]]\n").is_ok());
        let all = format!(
            "{:#}",
            parse("[[look]]\nweight = 0\n[[look]]\nweight = 0\n").unwrap_err()
        );
        assert!(
            all.contains("weight = 0") && all.contains("[[look]]"),
            "{all}"
        );
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
        assert!(with("hold = 8.0", "hold = -1.0").contains("camera.hold"));
        let none = format!("{SMALL}\n[camera]\ndutch = 0.2\n");
        let none = format!("{:#}", Manifest::parse(&none).unwrap_err());
        assert!(none.contains("at least one [[shot]]"), "{none}");
        let alone = format!("{SMALL}\n[[burst]]\nsize = \"close\"\n");
        let alone = format!("{:#}", Manifest::parse(&alone).unwrap_err());
        assert!(alone.contains("[[burst]] needs"), "{alone}");
        let pace = with("pace = \"calm\"", "pace = \"frantic\"");
        assert!(
            pace.contains("frantic") && pace.contains("sections"),
            "{pace}"
        );
    }

    #[test]
    fn a_bad_burst_is_named() {
        let with = |from: &str, to: &str| {
            let text = format!("{SMALL}{SHOTS}").replacen(from, to, 1);
            assert_ne!(
                text,
                format!("{SMALL}{SHOTS}"),
                "{from:?} is not in the shots"
            );
            format!("{:#}", Manifest::parse(&text).unwrap_err())
        };
        // A burst's cuts are hard: it has no move of its own.
        let moved = with("size = \"detail\"", "size = \"detail\"\nmove = \"glide\"");
        assert!(moved.contains("move") && moved.contains("size"), "{moved}");
        // Nor does it open a section or have a weight: only a `[[shot]]` is picked.
        for key in ["opens = true", "weight = 2"] {
            let text = with("size = \"detail\"", &format!("size = \"detail\"\n{key}"));
            assert!(text.contains(key.split(' ').next().unwrap()), "{text}");
        }
        assert!(with("size = \"detail\"", "size = \"huge\"").contains("huge"));
        let many = format!(
            "{SMALL}{SHOTS}{}",
            "[[burst]]\nsize = \"close\"\n".repeat(3)
        );
        let many = format!("{:#}", Manifest::parse(&many).unwrap_err());
        assert!(many.contains("at most 4 [[burst]]"), "{many}");
    }

    #[test]
    fn camera_fill_is_refused_with_a_word_about_bursts() {
        let text = format!("{SMALL}{SHOTS}").replacen("hold = 8.0", "hold = 8.0\nfill = true", 1);
        let message = format!("{:#}", Manifest::parse(&text).unwrap_err());
        assert!(
            message.contains("fill") && message.contains("[[burst]]"),
            "{message}"
        );
    }

    const INPUTS: &str = r##"
[[input]]
id = "surge"
label = { de = "Schub", en = "Surge" }
range = [0.0, 2.0]
rest = 0.5

[[input]]
id = "wave"

[[bind]]
target = "surge"
source = "hits"
how = "trigger"
when = { strength = 0.3, release = 0.5 }
fallback = "rest"

[[bind]]
target = "wave"
source = "drive"
how = "follow"
ease = "none"
fallback = { value = 0.25 }
"##;

    #[test]
    fn inputs_and_binds_are_read_with_defaults_filled_in() {
        let plain = Manifest::parse(SMALL).unwrap();
        assert!(plain.inputs.is_empty() && plain.binds.is_empty());
        let m = Manifest::parse(&format!("{SMALL}{INPUTS}")).unwrap();
        assert_eq!(m.inputs.len(), 2);
        assert_eq!(m.inputs[0].range, [0.0, 2.0]);
        assert_eq!(m.inputs[0].rest(), 0.5);
        assert_eq!(m.inputs[0].label.as_ref().unwrap().get("de"), "Schub");
        // The range is 0 to 1 and the rest its bottom when they are left out.
        assert_eq!((m.inputs[1].range, m.inputs[1].rest()), ([0.0, 1.0], 0.0));
        assert_eq!(m.binds[0].how, "trigger");
        assert_eq!(m.binds[0].fallback, Some(FallbackDecl::Word("rest".into())));
        assert_eq!(
            m.binds[1].fallback,
            Some(FallbackDecl::Value { value: 0.25 })
        );
        assert_eq!(m.binds[1].ease.as_deref(), Some("none"));
        assert!(m.binds[0].when.contains_key("strength"));
    }

    #[test]
    fn a_lab_replaces_the_inputs_and_binds_with_the_checks_of_the_manifest() {
        let m = Manifest::parse(&format!("{SMALL}{INPUTS}")).unwrap();
        // The text of the inputs as a lab reads them reads back as the same inputs.
        let text: String = m
            .inputs
            .iter()
            .map(|input| format!("[[input]]\n{}\n", input.declared()))
            .collect();
        assert!(
            text.contains("rest = 0.5") && text.contains("range = [0.0, 2.0]"),
            "{text}"
        );
        let only_inputs = m.with_bindings(&text).unwrap();
        assert_eq!(only_inputs.inputs, m.inputs);
        assert!(only_inputs.binds.is_empty());
        // No arrays means none; anything else is refused, as are the manifest's own mistakes.
        assert!(m.with_bindings("").unwrap().inputs.is_empty());
        let wrong = m.with_bindings("name = \"x\"").unwrap_err();
        assert!(format!("{wrong:#}").contains("name"), "{wrong:#}");
        let bind = "[[input]]\nid = \"a\"\n[[bind]]\ntarget = \"a\"\nsource = \"hits\"\nhow = \"follow\"\n";
        assert_eq!(m.with_bindings(bind).unwrap().binds.len(), 1);
        let refused = m
            .with_bindings(&bind.replace("hits", "loudness"))
            .unwrap_err();
        assert!(format!("{refused:#}").contains("loudness"), "{refused:#}");
        let orphan =
            m.with_bindings("[[bind]]\ntarget = \"a\"\nsource = \"hits\"\nhow = \"follow\"\n");
        assert!(format!("{:#}", orphan.unwrap_err()).contains("no input"));
    }

    #[test]
    fn the_example_of_the_docs_is_a_scene_that_reads() {
        let docs = include_str!("../../docs/music.md");
        let start =
            docs.find("### Example\n\n```toml\n").unwrap() + "### Example\n\n```toml\n".len();
        let example = &docs[start..start + docs[start..].find("```").unwrap()];
        let m = Manifest::parse(&format!("{SMALL}{example}")).unwrap();
        assert_eq!(m.inputs.len(), 3);
        assert_eq!(m.binds.len(), 3);
        // The input list of the docs' first example reads too.
        let first = docs.find("```toml\n[[input]]").unwrap() + "```toml\n".len();
        let first = &docs[first..first + docs[first..].find("```").unwrap()];
        Manifest::parse(&format!("{SMALL}{first}")).unwrap();
    }

    #[test]
    fn a_bad_input_or_bind_is_named() {
        let with = |from: &str, to: &str| {
            let all = format!("{SMALL}{INPUTS}");
            let text = all.replacen(from, to, 1);
            assert_ne!(text, all, "{from:?} is not in the inputs");
            format!("{:#}", Manifest::parse(&text).unwrap_err())
        };
        let twice = with("id = \"surge\"", "id = \"speed\"");
        assert!(
            twice.contains("speed") && twice.contains("twice"),
            "{twice}"
        );
        assert!(with("id = \"wave\"", "id = \"rv_wave\"").contains("rv_"));
        assert!(with("id = \"wave\"", "id = \"2wave\"").contains("not a valid name"));
        let target = with("target = \"wave\"", "target = \"gate\"");
        assert!(
            target.contains("`gate`") && target.contains("no input"),
            "{target}"
        );
        let double = with("target = \"wave\"", "target = \"surge\"");
        assert!(
            double.contains("`surge`") && double.contains("more than one"),
            "{double}"
        );
        let source = with("source = \"drive\"", "source = \"loudness\"");
        assert!(
            source.contains("`wave`") && source.contains("no source `loudness`"),
            "{source}"
        );
        let how = with("how = \"trigger\"", "how = \"fling\"");
        assert!(
            how.contains("`surge`") && how.contains("no how `fling`"),
            "{how}"
        );
        let key = with("strength = 0.3", "force = 0.3");
        assert!(
            key.contains("unknown key `force`") && key.contains("`surge`"),
            "{key}"
        );
        let fallback = with("fallback = \"rest\"", "fallback = \"drive\"");
        assert!(
            fallback.contains("fallback source that counts events"),
            "{fallback}"
        );
        assert!(
            with("fallback = \"rest\"", "fallback = \"mystery\"").contains("no fallback `mystery`")
        );
        assert!(with("range = [0.0, 2.0]", "range = [2.0, 0.0]").contains("range"));
        // A field the format does not have is a mistake, not ignored.
        let field = with("rest = 0.5", "rest = 0.5\nrestt = 1");
        assert!(field.contains("restt"), "{field}");
        let field = with("how = \"follow\"", "how = \"follow\"\nthen = 1");
        assert!(field.contains("then"), "{field}");
    }

    #[test]
    fn changing_only_the_binds_leaves_the_rest_equal_and_an_input_does_not() {
        let text = format!("{SMALL}{INPUTS}");
        let a = Manifest::parse(&text).unwrap();
        let b = Manifest::parse(&text.replace("strength = 0.3", "strength = 0.6")).unwrap();
        assert_ne!(a, b);
        assert_eq!(a.without_defaults(), b.without_defaults());
        let c = Manifest::parse(&text.replace("rest = 0.5", "rest = 1.5")).unwrap();
        assert_ne!(a.without_defaults(), c.without_defaults());
        let d = Manifest::parse(&format!("{text}\n[[input]]\nid = \"more\"\n")).unwrap();
        assert_ne!(a.without_defaults(), d.without_defaults());
    }

    #[test]
    fn effects_default_to_the_plain_ones_and_unknown_names_are_only_named() {
        let plain = Manifest::parse(SMALL).unwrap();
        assert_eq!(plain.effects.switch, ["fade", "dissolve"]);
        assert!(plain.effects.cut.is_empty() && plain.effects.lens.is_empty());
        assert!(plain.effects.unknown().is_empty());

        // Two lists, one per kind. Tables go after the top-level keys.
        let with = |table: &str| format!("{SMALL}\n[effects]\n{table}");
        let m = Manifest::parse(&with(
            "transitions = [\"fade\", \"flash\", \"cutin\", \"sparkle\"]\nlens = [\"flare\", \"swirl\"]\n",
        ))
        .unwrap();
        assert_eq!(m.effects.switch, ["fade", "flash", "cutin"]);
        assert_eq!(m.effects.cut, ["fade", "flash"]);
        assert_eq!(m.effects.lens, ["flare"]);
        assert_eq!(
            m.effects.unknown(),
            [
                "[effects] transitions = \"sparkle\"",
                "[effects] lens = \"swirl\""
            ]
        );
        // Either key alone.
        let only_lens = Manifest::parse(&with("lens = [\"leak\"]\n"))
            .unwrap()
            .effects;
        assert_eq!(only_lens.switch, ["fade", "dissolve"]);
        assert!(only_lens.cut.is_empty());
        assert_eq!(only_lens.lens, ["leak"]);
        let only_transitions = Manifest::parse(&with("transitions = [\"flash\"]\n"))
            .unwrap()
            .effects;
        assert_eq!(only_transitions.switch, ["fade", "dissolve", "flash"]);
        assert_eq!(only_transitions.cut, ["flash"]);
        assert!(only_transitions.lens.is_empty());
        // An empty table is the default.
        assert_eq!(
            Manifest::parse(&with("")).unwrap().effects,
            Effects::default()
        );
    }

    #[test]
    fn a_glitch_or_mosaic_is_a_transition_and_a_lens_effect_only_where_declared() {
        let with = |table: &str| format!("{SMALL}\n[effects]\n{table}");
        let lens_only = Manifest::parse(&with("lens = [\"glitch\", \"mosaic\"]\n"))
            .unwrap()
            .effects;
        assert!(lens_only.cut.is_empty() && !lens_only.switch.contains(&"glitch".to_string()));
        assert_eq!(lens_only.lens, ["glitch", "mosaic"]);
        let transitions_only = Manifest::parse(&with("transitions = [\"glitch\", \"mosaic\"]\n"))
            .unwrap()
            .effects;
        assert_eq!(transitions_only.cut, ["glitch", "mosaic"]);
        assert!(transitions_only.lens.is_empty());
    }

    #[test]
    fn a_name_of_the_other_kind_and_the_old_list_are_refused() {
        let with = |table: &str| format!("{SMALL}\n[effects]\n{table}");
        let message = Manifest::parse(&with("transitions = [\"flare\"]\n"))
            .unwrap_err()
            .to_string();
        assert!(
            message.contains("flare") && message.contains("lens = ["),
            "{message}"
        );
        let message = Manifest::parse(&with("lens = [\"flash\"]\n"))
            .unwrap_err()
            .to_string();
        assert!(
            message.contains("flash") && message.contains("transitions = ["),
            "{message}"
        );
        let old = SMALL.replacen(
            "format = 1\n",
            "format = 1\neffects = [\"fade\", \"flare\"]\n",
            1,
        );
        let message = Manifest::parse(&old).unwrap_err().to_string();
        assert!(
            message.contains("[effects]")
                && message.contains("transitions")
                && message.contains("lens"),
            "{message}"
        );
    }
}
