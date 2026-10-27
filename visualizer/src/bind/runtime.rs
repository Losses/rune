//! The runtime's own decisions as bindings: when an effect plays, when the camera cuts, when the
//! scene switches, how long an effect lasts and what moves in it. `runtime.toml` declares them in
//! the shape of a scene's `[[bind]]`; this module reads the file into one typed struct per area,
//! which the decision code (`moments`, `scene::director`, `switching`, `course`) asks instead of
//! keeping constants. See "The runtime's bindings" in `docs/music.md`.
//!
//! The file's values are the constants the code had before, so that behaviour at the defaults is
//! the same; a test pins that. A source is read through [`super::sources`], so a name means one
//! thing in a scene and here. Where a source means nothing at the frame's level the bind's
//! `fallback` takes over, and with none the decision is off there ([`Pick::said`]).

use std::sync::{Arc, LazyLock};

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;

use super::eval::Used;
use super::level::Level;
use super::sources::{self, Reading, Source};
use crate::inspect::InspectBinding;
use crate::moments::{Strength, key};
use crate::scene::{FallbackDecl, MusicValues};
use crate::switching::FOLLOWED;

/// The file the runtime starts with.
pub const EMBEDDED: &str = include_str!("../../runtime.toml");

/// Whether `source` means something in this frame's music. A source that exists only with a
/// followed beat (`bars`, `beats`) does when `locked` is above 0.5 (and the tempo is known), as
/// everywhere in the runtime; any other does where it means something at the frame's level.
pub fn counts_here(source: &Source, music: &MusicValues) -> bool {
    if source.levels == [true, false, false] {
        music.locked > FOLLOWED && music.tempo > 0.0
    } else {
        source.meaningful_at(Level::of(music))
    }
}

/// What a bind's means said in a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Said {
    /// The source means nothing here and there is no fallback: the decision is off.
    Off,
    Source(Reading),
    /// The fallback source of this name.
    Fallback(&'static Source, Reading),
    /// The fallback's fixed value.
    Value(f32),
}

impl Said {
    /// What was read, from whichever means: none when off. A fixed value has no events.
    pub fn reading(self) -> Option<Reading> {
        match self {
            Self::Off => None,
            Self::Source(reading) | Self::Fallback(_, reading) => Some(reading),
            Self::Value(value) => Some(Reading { value, count: 0 }),
        }
    }

    /// Whether an event came this frame, from whichever means.
    pub fn count(self) -> u32 {
        self.reading().map_or(0, |reading| reading.count)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Fb {
    Source(&'static Source),
    Value(f32),
}

/// A bind's source, the levels it is limited to (if any) and what it falls back to: what one
/// decision reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pick {
    pub(crate) source: &'static Source,
    pub(crate) fallback: Option<Fb>,
    pub(crate) levels: Option<[bool; 3]>,
}

impl Pick {
    /// A source with no fallback, read wherever it means something.
    fn plain(source: &'static Source) -> Self {
        Self {
            source,
            fallback: None,
            levels: None,
        }
    }

    /// The source the bind names.
    pub fn source(&self) -> &'static Source {
        self.source
    }

    /// The fixed value the bind falls back to, if that is what it does.
    pub fn fallback_value(&self) -> Option<f32> {
        match self.fallback {
            Some(Fb::Value(value)) => Some(value),
            _ => None,
        }
    }

    /// The source the bind falls back to, if that is what it does.
    pub fn fallback_source(&self) -> Option<&'static Source> {
        match self.fallback {
            Some(Fb::Source(source)) => Some(source),
            _ => None,
        }
    }

    /// Whether the bind's own source is the one read in this frame.
    pub fn applies(&self, music: &MusicValues) -> bool {
        match self.levels {
            Some(levels) => levels[Level::of(music).index()],
            None => counts_here(self.source, music),
        }
    }

    /// What the bind says in this frame's music.
    pub fn said(&self, music: &MusicValues) -> Said {
        if self.applies(music) {
            return Said::Source(self.source.read(music));
        }
        match self.fallback {
            Some(Fb::Source(source)) => Said::Fallback(source, source.read(music)),
            Some(Fb::Value(value)) => Said::Value(value),
            None => Said::Off,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The file

/// Which kind of source a target takes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// No source: the bind only holds numbers.
    None,
    /// An event source (it counts events).
    Event,
    /// A value source (an event source is read by its strength).
    Value,
    /// `bars` or `beats`, with a fixed value in seconds as the fallback.
    Counted,
    /// `bars`, with a fixed value in seconds as the fallback.
    Bars,
}

struct Spec {
    target: &'static str,
    hows: &'static [&'static str],
    kind: Kind,
    keys: &'static [&'static str],
    /// The fewest binds, and whether more than one is allowed.
    several: bool,
}

const TRIGGER: &[&str] = &["trigger"];
const FOLLOW: &[&str] = &["follow"];
const STEP: &[&str] = &["step"];

/// The targets: every decision the runtime opens.
const SPECS: &[Spec] = &[
    Spec {
        target: "moments.strong",
        hows: TRIGGER,
        kind: Kind::Event,
        keys: &["strength"],
        several: true,
    },
    Spec {
        target: "moments.medium",
        hows: TRIGGER,
        kind: Kind::Event,
        keys: &["strength"],
        several: true,
    },
    Spec {
        target: "moments.gap",
        hows: STEP,
        kind: Kind::Counted,
        keys: &["every"],
        several: false,
    },
    Spec {
        target: "director.cadence",
        hows: FOLLOW,
        kind: Kind::Value,
        keys: &["below", "above", "every", "seconds"],
        several: false,
    },
    Spec {
        target: "director.cut",
        hows: TRIGGER,
        kind: Kind::Event,
        keys: &["grace"],
        several: false,
    },
    Spec {
        target: "director.burst",
        hows: FOLLOW,
        kind: Kind::Value,
        keys: &["on", "from", "to", "lift"],
        several: false,
    },
    Spec {
        target: "switch.place",
        hows: TRIGGER,
        kind: Kind::Event,
        keys: &["levels", "strength", "lift", "change", "any"],
        several: true,
    },
    Spec {
        target: "switch.early",
        hows: FOLLOW,
        kind: Kind::Value,
        keys: &[],
        several: false,
    },
    Spec {
        target: "switch.timing",
        hows: &[],
        kind: Kind::None,
        keys: &["min", "lead", "relax", "max", "grace"],
        several: false,
    },
    Spec {
        target: "switch.forced",
        hows: TRIGGER,
        kind: Kind::Event,
        keys: &[],
        several: true,
    },
    Spec {
        target: "course.length",
        hows: STEP,
        kind: Kind::Bars,
        keys: &["phrase", "least", "most"],
        several: false,
    },
    Spec {
        target: "course.amount",
        hows: FOLLOW,
        kind: Kind::Value,
        keys: &["least"],
        several: false,
    },
    Spec {
        target: "course.motion",
        hows: STEP,
        kind: Kind::Event,
        keys: &["phase", "push_least", "beat_seconds"],
        several: false,
    },
];

/// The names of the targets, for messages.
pub fn targets() -> String {
    SPECS
        .iter()
        .map(|spec| spec.target)
        .collect::<Vec<_>>()
        .join(", ")
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default, rename = "bind")]
    binds: Vec<Raw>,
}

/// One `[[bind]]` as written.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    target: String,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    how: Option<String>,
    #[serde(default)]
    when: toml::Table,
    #[serde(default)]
    fallback: Option<FallbackDecl>,
}

/// A bind that was checked against its target.
struct Bound {
    spec: &'static Spec,
    how: String,
    when: toml::Table,
    pick: Option<Pick>,
    /// For messages: "[[bind]] for `target` (source `x`)".
    name: String,
}

fn number(value: &toml::Value, what: &str) -> Result<f32> {
    let v = match value {
        toml::Value::Float(v) => *v as f32,
        toml::Value::Integer(v) => *v as f32,
        other => bail!("{what} must be a number, not {other}"),
    };
    ensure!(v.is_finite(), "{what} must be a finite number");
    Ok(v)
}

impl Bound {
    fn new(raw: Raw) -> Result<Self> {
        let spec = SPECS
            .iter()
            .find(|spec| spec.target == raw.target)
            .with_context(|| {
                format!(
                    "[[bind]] for `{}`: no such target (the targets are: {})",
                    raw.target,
                    targets()
                )
            })?;
        let name = match &raw.source {
            Some(source) => format!("[[bind]] for `{}` (source `{source}`)", raw.target),
            None => format!("[[bind]] for `{}`", raw.target),
        };
        Self::check(raw, spec, &name).with_context(|| name.clone())
    }

    fn check(raw: Raw, spec: &'static Spec, name: &str) -> Result<Self> {
        for key in raw.when.keys() {
            ensure!(
                spec.keys.contains(&key.as_str()),
                "unknown key `{key}` in `when` (`{}` takes: {})",
                spec.target,
                if spec.keys.is_empty() {
                    "none".to_string()
                } else {
                    spec.keys.join(", ")
                }
            );
        }
        let how = match (&raw.how, spec.hows) {
            (None, []) => String::new(),
            (None, hows) => bail!(
                "`how` is missing (`{}` takes: {})",
                spec.target,
                hows.join(", ")
            ),
            (Some(how), hows) if hows.contains(&how.as_str()) => how.clone(),
            (Some(how), []) => bail!("`{}` takes no `how`, not `{how}`", spec.target),
            (Some(how), hows) => bail!(
                "no `how` `{how}` for `{}` (it takes: {})",
                spec.target,
                hows.join(", ")
            ),
        };
        let pick = match (&raw.source, spec.kind) {
            (None, Kind::None) => {
                ensure!(
                    raw.fallback.is_none(),
                    "`{}` takes no fallback",
                    spec.target
                );
                None
            }
            (Some(_), Kind::None) => bail!("`{}` takes no source", spec.target),
            (None, _) => bail!("`source` is missing"),
            (Some(source), kind) => {
                let source = find(source)?;
                ensure!(
                    kind != Kind::Event || source.event,
                    "`{}` needs a source that counts events, not `{}`",
                    spec.target,
                    source.name
                );
                ensure!(
                    kind != Kind::Counted || ["bars", "beats"].contains(&source.name),
                    "`{}` counts `bars` or `beats`, not `{}`",
                    spec.target,
                    source.name
                );
                ensure!(
                    kind != Kind::Bars || source.name == "bars",
                    "`{}` counts `bars`, not `{}`",
                    spec.target,
                    source.name
                );
                let fallback = match &raw.fallback {
                    None => None,
                    Some(FallbackDecl::Value { value }) => {
                        ensure!(
                            value.is_finite(),
                            "`fallback` value must be a finite number"
                        );
                        Some(Fb::Value(*value))
                    }
                    Some(FallbackDecl::Word(word)) => {
                        ensure!(
                            word != "hold" && word != "rest",
                            "`fallback` `{word}` makes no sense for a decision (a source or {{ value = x }})"
                        );
                        let fallback = find(word)?;
                        ensure!(
                            !matches!(kind, Kind::Counted | Kind::Bars),
                            "`{}` takes a fallback of {{ value = seconds }}, not a source",
                            spec.target
                        );
                        ensure!(
                            kind != Kind::Event || fallback.event,
                            "the fallback of `{}` must count events, not `{word}`",
                            spec.target
                        );
                        Some(Fb::Source(fallback))
                    }
                };
                let levels = match raw.when.get("levels") {
                    None => None,
                    Some(toml::Value::String(letters)) => Some(levels_of(letters)?),
                    Some(other) => {
                        bail!("`when.levels` must be letters such as \"AB\", not {other}")
                    }
                };
                Some(Pick {
                    source,
                    fallback,
                    levels,
                })
            }
        };
        Ok(Self {
            spec,
            how,
            when: raw.when,
            pick,
            name: name.to_string(),
        })
    }

    fn pick(&self) -> Pick {
        self.pick.expect("a bind with a source")
    }

    fn num(&self, key: &str, default: f32) -> Result<f32> {
        match self.when.get(key) {
            Some(value) => number(value, &format!("`when.{key}`")),
            None => Ok(default),
        }
        .with_context(|| self.name.clone())
    }

    fn flag(&self, key: &str, default: bool) -> Result<bool> {
        match self.when.get(key) {
            None => Ok(default),
            Some(toml::Value::Boolean(v)) => Ok(*v),
            Some(other) => Err(anyhow::anyhow!(
                "`when.{key}` must be true or false, not {other}"
            )),
        }
        .with_context(|| self.name.clone())
    }

    fn triple(&self, key: &str, default: [f32; 3]) -> Result<[f32; 3]> {
        let what = format!("`when.{key}`");
        let result = (|| match self.when.get(key) {
            None => Ok(default),
            Some(toml::Value::Array(items)) if items.len() == 3 => Ok([
                number(&items[0], &what)?,
                number(&items[1], &what)?,
                number(&items[2], &what)?,
            ]),
            Some(_) => bail!("{what} must be three numbers: [quiet, normal, high]"),
        })();
        result.with_context(|| self.name.clone())
    }

    /// A source named in `when`, which must be an event source if `event`.
    fn source_in(&self, key: &str, default: &str, event: bool) -> Result<&'static Source> {
        let result = (|| {
            let name = match self.when.get(key) {
                None => default,
                Some(toml::Value::String(name)) => name,
                Some(other) => bail!("`when.{key}` must be the name of a source, not {other}"),
            };
            let source = find(name)?;
            ensure!(
                !event || source.event,
                "`when.{key}` needs a source that counts events, not `{name}`"
            );
            Ok(source)
        })();
        result.with_context(|| self.name.clone())
    }

    fn require(&self, ok: bool, message: &str) -> Result<()> {
        ensure!(ok, "{}: {message}", self.name);
        Ok(())
    }
}

fn find(name: &str) -> Result<&'static Source> {
    sources::find(name)
        .with_context(|| format!("no source `{name}` (the sources are: {})", sources::names()))
}

fn levels_of(letters: &str) -> Result<[bool; 3]> {
    let mut levels = [false; 3];
    for letter in letters.chars() {
        match letter {
            'A' => levels[0] = true,
            'B' => levels[1] = true,
            'C' => levels[2] = true,
            other => bail!("no level `{other}` in `when.levels` (A, B or C)"),
        }
    }
    ensure!(levels != [false; 3], "`when.levels` names no level");
    Ok(levels)
}

// ---------------------------------------------------------------------------------------------
// The areas

/// One bind that counts as a moment when its event is at least `strength`.
#[derive(Clone, Debug, PartialEq)]
pub struct Trigger {
    pub(crate) pick: Pick,
    pub(crate) strength: f32,
}

impl Trigger {
    fn of(bound: &Bound) -> Result<Self> {
        Ok(Self {
            pick: bound.pick(),
            strength: bound.num("strength", 0.0)?,
        })
    }

    /// The event of this frame, if it came and is strong enough.
    fn fires(&self, music: &MusicValues) -> Option<Reading> {
        self.pick
            .said(music)
            .reading()
            .filter(|reading| reading.count > 0 && reading.value >= self.strength)
    }
}

/// `moments.*`: when an effect may play (see [`crate::moments`]).
#[derive(Clone, Debug, PartialEq)]
pub struct MomentBinds {
    pub(crate) strong: Vec<Trigger>,
    pub(crate) medium: Vec<Trigger>,
    pub(crate) gap: Pick,
    /// How many of the gap's source (bars, or beats) an effect keeps the next medium one away.
    pub(crate) gap_every: f32,
}

impl MomentBinds {
    fn new(binds: &[&Bound]) -> Result<Self> {
        let of = |target: &str| -> Result<Vec<Trigger>> {
            binds
                .iter()
                .filter(|b| b.spec.target == target)
                .map(|b| Trigger::of(b))
                .collect()
        };
        let gap = binds
            .iter()
            .find(|b| b.spec.target == "moments.gap")
            .expect("checked");
        let gap_every = gap.num("every", 8.0)?;
        gap.require(gap_every > 0.0, "`when.every` must be above 0")?;
        Ok(Self {
            strong: of("moments.strong")?,
            medium: of("moments.medium")?,
            gap: gap.pick(),
            gap_every,
        })
    }

    /// Whether this frame starts a section: a cue of the first `moments.medium` bind (a strong
    /// moment is one too). This is what the director, the effects' phrase and the switch call a
    /// section start.
    pub fn section_start(&self, music: &MusicValues) -> bool {
        self.medium[0].fires(music).is_some()
    }

    /// The moment of a frame, if it is one, and the key its roll is made from: its section, and
    /// for a medium moment that is not a section start (an impact), its number `which` in the
    /// section.
    pub fn moment_of(&self, music: &MusicValues, which: u32) -> Option<(Strength, u32)> {
        let section = music.section.max(0.0) as u32;
        if self.strong.iter().any(|t| t.fires(music).is_some()) {
            return Some((Strength::Strong, key(section, 0)));
        }
        let medium = self.medium.iter().find(|t| t.fires(music).is_some())?;
        let which = if medium.pick.source.name == "cues" {
            0
        } else {
            which
        };
        Some((Strength::Medium, key(section, which)))
    }

    /// Whether an event of a medium moment that is not a section start (an impact) came this
    /// frame, for the count of them in the section.
    pub fn counts_other(&self, music: &MusicValues) -> bool {
        self.medium
            .iter()
            .filter(|t| t.pick.source.name != "cues")
            .any(|t| t.pick.said(music).count() > 0)
    }

    /// How long a medium moment keeps away after an effect, in seconds: `every` bars of the music
    /// while its beat is followed, else of the fallback's seconds. None where the gap is off.
    pub fn gap_seconds(&self, music: &MusicValues) -> Option<f32> {
        match self.gap.said(music) {
            Said::Source(_) => {
                let beat = 60.0 / music.tempo;
                let unit = if self.gap.source.name == "beats" {
                    beat
                } else {
                    music.meter.max(1.0) * beat
                };
                Some(self.gap_every * unit)
            }
            Said::Value(seconds) => Some(self.gap_every * seconds),
            Said::Fallback(..) | Said::Off => None,
        }
    }
}

/// `director.*`: how often the camera cuts, what a cut lands on, and the burst before a lift.
#[derive(Clone, Debug, PartialEq)]
pub struct DirectorBinds {
    pub(crate) cadence: Pick,
    pub(crate) below: f32,
    pub(crate) above: f32,
    pub(crate) every: [u32; 3],
    pub(crate) seconds: [f32; 3],
    pub(crate) cut: Pick,
    pub grace: f32,
    pub(crate) burst: Pick,
    pub(crate) on: Pick,
    pub(crate) from: &'static Source,
    pub(crate) to: &'static Source,
    pub(crate) lift: f32,
}

/// What the burst sees in a frame that is a beat before a section (see
/// [`DirectorBinds::burst`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ahead {
    /// Seconds to the next section, -1 when unknown.
    pub cue_in: f32,
    /// Whether the next section is louder by the lift.
    pub lifts: bool,
}

impl DirectorBinds {
    fn new(binds: &[&Bound]) -> Result<Self> {
        let one = |target: &str| {
            *binds
                .iter()
                .find(|b| b.spec.target == target)
                .expect("checked")
        };
        let cadence = one("director.cadence");
        let every = cadence.triple("every", [8.0, 4.0, 2.0])?;
        cadence.require(
            every.iter().all(|e| *e >= 1.0 && e.fract() == 0.0),
            "`when.every` must be whole numbers from 1",
        )?;
        let seconds = cadence.triple("seconds", [24.0, 12.0, 6.0])?;
        cadence.require(
            seconds.iter().all(|s| *s > 0.0),
            "`when.seconds` must be above 0",
        )?;
        let (below, above) = (cadence.num("below", 0.35)?, cadence.num("above", 0.65)?);
        cadence.require(
            below <= above,
            "`when.below` must not be above `when.above`",
        )?;
        let cut = one("director.cut");
        let grace = cut.num("grace", 2.0)?;
        cut.require(grace >= 0.0, "`when.grace` must not be negative")?;
        let burst = one("director.burst");
        Ok(Self {
            cadence: cadence.pick(),
            below,
            above,
            every: every.map(|e| e as u32),
            seconds,
            cut: cut.pick(),
            grace,
            burst: burst.pick(),
            on: Pick::plain(burst.source_in("on", "beats", true)?),
            from: burst.source_in("from", "section_energy", false)?,
            to: burst.source_in("to", "next_energy", false)?,
            lift: burst.num("lift", 0.1)?,
        })
    }

    /// How lively the section is: 0 quiet, 1 normal, 2 high. Normal where the cadence's source
    /// says nothing.
    pub fn tier(&self, music: &MusicValues) -> usize {
        let Some(energy) = self.cadence.said(music).reading() else {
            return 1;
        };
        if energy.value < self.below {
            0
        } else if energy.value > self.above {
            2
        } else {
            1
        }
    }

    /// Events of `director.cut` between two cuts with a followed beat, in a tier.
    pub fn every(&self, tier: usize) -> u32 {
        self.every[tier]
    }

    /// Seconds between two cuts without, in a tier.
    pub fn seconds(&self, tier: usize) -> f32 {
        self.seconds[tier]
    }

    /// What a cut lands on in this frame: its source, or the fallback's, or nothing.
    pub fn cut(&self, music: &MusicValues) -> Said {
        self.cut.said(music)
    }

    /// The burst before a lift, if this frame is a beat it can cut on: the source of
    /// `director.burst` says something, `on` has an event, and the tempo is known.
    pub fn burst(&self, music: &MusicValues) -> Option<Ahead> {
        if music.tempo <= 0.0 || self.on.said(music).count() == 0 {
            return None;
        }
        let cue_in = self.burst.said(music).reading()?.value;
        Some(Ahead {
            cue_in,
            lifts: self.to.read(music).value - self.from.read(music).value >= self.lift,
        })
    }
}

/// One `switch.place` bind.
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub(crate) pick: Pick,
    pub(crate) strength: f32,
    pub(crate) lift: f32,
    pub(crate) change: f32,
    /// Any event is a place (no section is known to differ).
    pub(crate) any: bool,
}

/// What a place heard in a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Heard {
    pub strength: f32,
    /// The least strength of a section start, and of a lift.
    pub section_at: f32,
    pub lift_at: f32,
    /// Any event is a place.
    pub any: bool,
}

/// `switch.*`: where scenes switch and how long they wait.
#[derive(Clone, Debug, PartialEq)]
pub struct SwitchBinds {
    pub(crate) places: Vec<Place>,
    pub(crate) early: Pick,
    pub(crate) forced: Vec<Pick>,
    pub min: f64,
    pub lead: f64,
    pub relax: f64,
    pub max: f64,
    pub grace: f64,
}

impl SwitchBinds {
    fn new(binds: &[&Bound]) -> Result<Self> {
        let of = |target: &'static str| binds.iter().filter(move |b| b.spec.target == target);
        let places = of("switch.place")
            .map(|b| {
                Ok(Place {
                    pick: b.pick(),
                    strength: b.num("strength", 0.0)?,
                    lift: b.num("lift", 0.8)?,
                    change: b.num("change", 0.1)?,
                    any: b.flag("any", false)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let timing = of("switch.timing").next().expect("checked");
        let seconds = |key: &str, default: f32| -> Result<f64> {
            let value = timing.num(key, default)?;
            timing.require(value >= 0.0, &format!("`when.{key}` must not be negative"))?;
            Ok(f64::from(value))
        };
        let (min, lead, relax, max, grace) = (
            seconds("min", 30.0)?,
            seconds("lead", 8.0)?,
            seconds("relax", 60.0)?,
            seconds("max", 90.0)?,
            seconds("grace", 2.0)?,
        );
        timing.require(lead <= min, "`when.lead` must not be more than `when.min`")?;
        timing.require(
            min <= relax && relax <= max,
            "the times must go min, relax, max",
        )?;
        Ok(Self {
            places,
            early: of("switch.early").next().expect("checked").pick(),
            forced: of("switch.forced").map(|b| b.pick()).collect(),
            min,
            lead,
            relax,
            max,
            grace,
        })
    }

    /// What the places hear in this frame: the first that applies at the frame's level and has an
    /// event.
    pub fn heard(&self, music: &MusicValues) -> Option<Heard> {
        self.places.iter().find_map(|place| {
            let reading = place.pick.said(music).reading()?;
            (reading.count > 0).then_some(Heard {
                strength: reading.value,
                section_at: place.strength,
                lift_at: place.lift,
                any: place.any,
            })
        })
    }

    /// How much a section's energy must change for it to be another section, or a lift or drop
    /// (from the first place that tells sections apart).
    pub fn change(&self) -> f32 {
        self.places
            .iter()
            .find(|place| !place.any)
            .map_or(0.1, |place| place.change)
    }

    /// The seconds to the next section start, if it is known and the early fade may use it.
    pub fn early(&self, music: &MusicValues) -> Option<f32> {
        self.early
            .said(music)
            .reading()
            .map(|reading| reading.value)
            .filter(|seconds| *seconds >= 0.0)
    }

    /// Whether an event that ends the wait came this frame.
    pub fn forced(&self, music: &MusicValues) -> bool {
        self.forced.iter().any(|pick| pick.said(music).count() > 0)
    }
}

/// `course.*`: how long the long effects last, how much of them is on, and what moves in them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CourseBinds {
    pub(crate) length: Pick,
    /// The phrase, and the least and the most of it a course lasts, in bars.
    pub phrase: u32,
    pub least_bars: u32,
    pub most_bars: u32,
    pub(crate) amount: Pick,
    /// The music's least part of the amount.
    pub least: f32,
    pub(crate) motion: Pick,
    pub(crate) phase: &'static Source,
    /// The least push of a beat with no hit, and how long a beat is without a followed one.
    pub push_least: f32,
    pub beat_seconds: f32,
}

impl CourseBinds {
    fn new(binds: &[&Bound]) -> Result<Self> {
        let one = |target: &str| {
            *binds
                .iter()
                .find(|b| b.spec.target == target)
                .expect("checked")
        };
        let length = one("course.length");
        let whole = |key: &str, default: f32| -> Result<u32> {
            let value = length.num(key, default)?;
            length.require(
                value >= 1.0 && value.fract() == 0.0,
                &format!("`when.{key}` must be a whole number from 1"),
            )?;
            Ok(value as u32)
        };
        let (phrase, least_bars, most_bars) = (
            whole("phrase", 4.0)?,
            whole("least", 2.0)?,
            whole("most", 8.0)?,
        );
        length.require(
            least_bars <= most_bars,
            "`when.least` must not be above `when.most`",
        )?;
        let amount = one("course.amount");
        let least = amount.num("least", 0.35)?;
        amount.require(
            (0.0..=1.0).contains(&least),
            "`when.least` must be from 0 to 1",
        )?;
        let motion = one("course.motion");
        let push_least = motion.num("push_least", 0.3)?;
        motion.require(
            (0.0..=1.0).contains(&push_least),
            "`when.push_least` must be from 0 to 1",
        )?;
        let beat_seconds = motion.num("beat_seconds", 0.5)?;
        motion.require(beat_seconds > 0.0, "`when.beat_seconds` must be above 0")?;
        Ok(Self {
            length: length.pick(),
            phrase,
            least_bars,
            most_bars,
            amount: amount.pick(),
            least,
            motion: motion.pick(),
            phase: motion.source_in("phase", "clock", false)?,
            push_least,
            beat_seconds,
        })
    }

    /// What `course.length` says: counted in bars while the beat is followed, else a time.
    pub fn length(&self, music: &MusicValues) -> Said {
        self.length.said(music)
    }

    /// The music's part of the amount in this frame, 0 to 1 (0 where the source says nothing).
    pub fn amount(&self, music: &MusicValues) -> f32 {
        self.amount
            .said(music)
            .reading()
            .map_or(0.0, |reading| reading.value.clamp(0.0, 1.0))
    }

    /// The beat time the motion starts in phase with.
    pub fn phase(&self, music: &MusicValues) -> f32 {
        self.phase.read(music).value.max(0.0).fract()
    }

    /// What drives the motion in this frame: the beat's own source while it is followed (then
    /// [`Said::Source`]), else the events of the fallback that push it.
    pub fn motion(&self, music: &MusicValues) -> Said {
        self.motion.said(music)
    }
}

// ---------------------------------------------------------------------------------------------
// All of it

/// One bind of the file, for the lab to show.
#[derive(Clone, Debug)]
struct Row {
    target: &'static str,
    how: String,
    pick: Option<Pick>,
}

/// The runtime's decisions, read from a file of `[[bind]]`s.
#[derive(Clone, Debug)]
pub struct RuntimeBindings {
    pub moments: Arc<MomentBinds>,
    pub director: Arc<DirectorBinds>,
    pub switching: Arc<SwitchBinds>,
    pub course: CourseBinds,
    rows: Vec<Row>,
    text: String,
}

static EMBEDDED_BINDINGS: LazyLock<Arc<RuntimeBindings>> = LazyLock::new(|| {
    Arc::new(
        RuntimeBindings::parse(EMBEDDED)
            .unwrap_or_else(|e| panic!("the embedded runtime.toml is wrong: {e:#}")),
    )
});

impl RuntimeBindings {
    /// The bindings of the file the visualizer was built with.
    pub fn embedded() -> Arc<Self> {
        EMBEDDED_BINDINGS.clone()
    }

    /// Read the bindings from the text of a file. Fails, naming the bind, for a target, source,
    /// `how`, `when` key or value that does not exist or makes no sense, and for a target left
    /// without a bind.
    pub fn parse(text: &str) -> Result<Self> {
        let file: File = toml::from_str(text).context("runtime.toml")?;
        let bound = file
            .binds
            .into_iter()
            .map(Bound::new)
            .collect::<Result<Vec<_>>>()?;
        for spec in SPECS {
            let n = bound
                .iter()
                .filter(|b| b.spec.target == spec.target)
                .count();
            ensure!(
                n >= 1,
                "runtime.toml has no [[bind]] for `{}`, which every decision needs",
                spec.target
            );
            ensure!(
                n == 1 || spec.several,
                "runtime.toml has {n} [[bind]]s for `{}`, which takes one",
                spec.target
            );
        }
        let area = |prefix: &str| -> Vec<&Bound> {
            bound
                .iter()
                .filter(|b| b.spec.target.starts_with(prefix))
                .collect()
        };
        Ok(Self {
            moments: Arc::new(MomentBinds::new(&area("moments."))?),
            director: Arc::new(DirectorBinds::new(&area("director."))?),
            switching: Arc::new(SwitchBinds::new(&area("switch."))?),
            course: CourseBinds::new(&area("course."))?,
            rows: bound
                .iter()
                .map(|b| Row {
                    target: b.spec.target,
                    how: b.how.clone(),
                    pick: b.pick,
                })
                .collect(),
            text: text.to_string(),
        })
    }

    /// The text these were read from.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// What each bind says in `music`, for a lab: the level, which means gave the value (a
    /// decision that is off shows as the rest), the source's own value and the value read.
    pub fn report(&self, music: &MusicValues) -> Vec<InspectBinding> {
        let level = Level::of(music);
        self.rows
            .iter()
            .map(|row| {
                let Some(pick) = row.pick else {
                    return InspectBinding {
                        target: row.target.to_string(),
                        source: String::new(),
                        how: row.how.clone(),
                        level,
                        used: Used::Value,
                        source_value: 0.0,
                        output: 0.0,
                    };
                };
                let said = pick.said(music);
                InspectBinding {
                    target: row.target.to_string(),
                    source: pick.source.name.to_string(),
                    how: row.how.clone(),
                    level,
                    used: match said {
                        Said::Off => Used::Rest,
                        Said::Source(_) => Used::Source,
                        Said::Fallback(source, _) => Used::Fallback(source.name),
                        Said::Value(_) => Used::Value,
                    },
                    source_value: pick.source.read(music).value,
                    output: said.reading().map_or(0.0, |reading| reading.value),
                }
            })
            .collect()
    }
}

impl Default for RuntimeBindings {
    fn default() -> Self {
        (*Self::embedded()).clone()
    }
}

impl Default for MomentBinds {
    fn default() -> Self {
        (*RuntimeBindings::embedded().moments).clone()
    }
}

impl Default for DirectorBinds {
    fn default() -> Self {
        (*RuntimeBindings::embedded().director).clone()
    }
}

impl Default for SwitchBinds {
    fn default() -> Self {
        (*RuntimeBindings::embedded().switching).clone()
    }
}

impl Default for CourseBinds {
    fn default() -> Self {
        RuntimeBindings::embedded().course
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The embedded file with `from` (which must be in it) replaced by `to`.
    pub(crate) fn embedded_with(from: &str, to: &str) -> RuntimeBindings {
        assert!(EMBEDDED.contains(from), "not in runtime.toml: {from}");
        RuntimeBindings::parse(&EMBEDDED.replacen(from, to, 1)).unwrap()
    }

    fn error_of(from: &str, to: &str) -> String {
        assert!(EMBEDDED.contains(from), "not in runtime.toml: {from}");
        let error = RuntimeBindings::parse(&EMBEDDED.replacen(from, to, 1))
            .expect_err("the text should be refused");
        format!("{error:#}")
    }

    #[test]
    fn the_embedded_file_parses_and_has_every_target() {
        let runtime = RuntimeBindings::parse(EMBEDDED).unwrap();
        assert_eq!(runtime.text(), EMBEDDED);
        let report = runtime.report(&MusicValues::default());
        for spec in SPECS {
            assert!(
                report.iter().any(|row| row.target == spec.target),
                "no row for {}",
                spec.target
            );
        }
        // Several binds on one target: the ORs.
        let count = |target: &str| report.iter().filter(|r| r.target == target).count();
        assert_eq!(count("moments.medium"), 2);
        assert_eq!(count("switch.place"), 2);
        assert_eq!(count("switch.forced"), 2);
    }

    #[test]
    fn a_mistake_names_the_bind() {
        let cadence = "target = \"director.cadence\"";
        let message = error_of(cadence, "target = \"director.cadense\"");
        assert!(
            message.contains("director.cadense") && message.contains("targets are"),
            "{message}"
        );
        let message = error_of("source = \"section_energy\"", "source = \"loudness\"");
        assert!(
            message.contains("director.cadence") && message.contains("loudness"),
            "{message}"
        );
        let message = error_of("when = { strength = 0.8 }", "when = { strenght = 0.8 }");
        assert!(
            message.contains("moments.strong") && message.contains("strenght"),
            "{message}"
        );
        let message = error_of("fallback = \"drive\"", "fallback = \"hold\"");
        assert!(
            message.contains("director.cadence") && message.contains("hold"),
            "{message}"
        );
        // An event target needs an event source.
        let message = error_of(
            "target = \"director.cut\"\nsource = \"bars\"",
            "target = \"director.cut\"\nsource = \"drive\"",
        );
        assert!(
            message.contains("director.cut") && message.contains("events"),
            "{message}"
        );
        let message = error_of(
            "how = \"follow\"\nwhen = { below",
            "how = \"trigger\"\nwhen = { below",
        );
        assert!(
            message.contains("director.cadence") && message.contains("how"),
            "{message}"
        );
        let message = error_of("levels = \"C\"", "levels = \"D\"");
        assert!(
            message.contains("switch.place") && message.contains("level"),
            "{message}"
        );
        // A target left without a bind.
        let without = EMBEDDED.replacen(
            "target = \"course.amount\"",
            "target = \"course.amount2\"",
            1,
        );
        assert!(RuntimeBindings::parse(&without).is_err());
        let message = format!("{:#}", RuntimeBindings::parse("").err().unwrap());
        assert!(message.contains("no [[bind]]"), "{message}");
    }

    #[test]
    fn a_source_means_something_where_the_standard_says() {
        let at = |level| match level {
            Level::A => MusicValues {
                has_chart: 1,
                locked: 1.0,
                tempo: 120.0,
                ..MusicValues::default()
            },
            Level::B => MusicValues {
                has_chart: 1,
                ..MusicValues::default()
            },
            Level::C => MusicValues::default(),
        };
        let cadence = RuntimeBindings::embedded().director.cadence;
        // The chart's energy where there is a chart, else the fallback.
        assert!(matches!(cadence.said(&at(Level::A)), Said::Source(_)));
        assert!(matches!(cadence.said(&at(Level::B)), Said::Source(_)));
        assert!(matches!(
            cadence.said(&at(Level::C)),
            Said::Fallback(source, _) if source.name == "drive"
        ));
        // `bars` count only with a followed beat.
        let gap = RuntimeBindings::embedded().moments.gap;
        assert!(matches!(gap.said(&at(Level::A)), Said::Source(_)));
        assert_eq!(gap.said(&at(Level::B)), Said::Value(2.0));
        // A bind with no fallback is off where its source means nothing.
        let burst = RuntimeBindings::embedded().director.burst;
        assert_eq!(burst.said(&at(Level::C)), Said::Off);
    }

    #[test]
    fn the_report_shows_the_level_and_which_means_gave_the_value() {
        let runtime = RuntimeBindings::embedded();
        let music = MusicValues {
            drive: 0.8,
            ..MusicValues::default()
        };
        let report = runtime.report(&music);
        let cadence = report
            .iter()
            .find(|r| r.target == "director.cadence")
            .unwrap();
        assert_eq!(cadence.level, Level::C);
        assert_eq!(cadence.used, Used::Fallback("drive"));
        assert_eq!(cadence.output, 0.8);
        assert_eq!(cadence.source, "section_energy");
        let burst = report
            .iter()
            .find(|r| r.target == "director.burst")
            .unwrap();
        assert_eq!(burst.used, Used::Rest, "off at level C");
        let gap = report.iter().find(|r| r.target == "moments.gap").unwrap();
        assert_eq!(gap.used, Used::Value);
        assert_eq!(gap.output, 2.0);
    }
}
