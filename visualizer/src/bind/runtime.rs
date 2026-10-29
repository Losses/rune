//! The runtime's own decisions as bindings: when an effect plays, when the camera cuts, how long an
//! effect lasts and what moves in it. `runtime.toml` declares them in the shape of a scene's
//! `[[bind]]`; this module reads the file into one typed struct per area, which the decision code
//! (`moments`, `scene::director`, `course`) asks instead of keeping constants. See "The runtime's
//! bindings" in `docs/music.md`.
//!
//! The file's values are the constants the code had before, so that behaviour at the defaults is
//! the same; a test pins that. A source is read through [`super::sources`], so a name means one
//! thing in a scene and here. Where a source means nothing at the frame's level the bind's
//! `fallback` takes over, and with none the decision is off there ([`Pick::said`]).

use std::sync::{Arc, LazyLock};

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;

use super::declared;
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

/// A bind's source and what it falls back to: what one decision reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pick {
    pub(crate) source: &'static Source,
    pub(crate) fallback: Option<Fb>,
}

impl Pick {
    /// A source with no fallback, read wherever it means something.
    fn plain(source: &'static Source) -> Self {
        Self {
            source,
            fallback: None,
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
        counts_here(self.source, music)
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
        keys: &["below", "above", "seconds", "gap", "lead"],
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
        target: "director.sizes",
        hows: &[],
        kind: Kind::None,
        keys: &["quiet", "normal", "high"],
        several: false,
    },
    Spec {
        target: "director.burst",
        hows: FOLLOW,
        kind: Kind::Value,
        keys: &["on", "from", "to", "lift", "chance"],
        several: false,
    },
    Spec {
        target: "look.change",
        hows: TRIGGER,
        kind: Kind::Event,
        keys: &[
            "strength",
            "gap",
            "fade",
            "fade_seconds",
            "strength_unlabelled",
            "gap_unlabelled",
        ],
        several: false,
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
    /// As written, for the lab.
    fallback: Option<FallbackDecl>,
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
                Some(Pick { source, fallback })
            }
        };
        Ok(Self {
            spec,
            how,
            when: raw.when,
            pick,
            fallback: raw.fallback,
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

    /// Five numbers in `when`, finite and 0 or more and not all 0 (the director's wish for each
    /// size, see `director.sizes`).
    fn wishes(&self, key: &str, default: [f32; 5]) -> Result<[f32; 5]> {
        let what = format!("`when.{key}`");
        let result = (|| match self.when.get(key) {
            None => Ok(default),
            Some(toml::Value::Array(items)) if items.len() == 5 => {
                let mut wishes = [0.0; 5];
                for (wish, item) in wishes.iter_mut().zip(items) {
                    *wish = number(item, &what)?;
                    ensure!(*wish >= 0.0, "{what} must not hold a negative number");
                }
                ensure!(wishes.iter().any(|w| *w > 0.0), "{what} must not be all 0");
                Ok(wishes)
            }
            Some(_) => bail!("{what} must be five numbers: [wide, medium, close, detail, top]"),
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
    pub(crate) seconds: [f32; 3],
    pub(crate) gap: f32,
    pub(crate) lead: f32,
    /// How much each size is wanted in a section of each tier, in the order wide, medium, close,
    /// detail, top.
    pub(crate) sizes: [[f32; 5]; 3],
    pub(crate) cut: Pick,
    pub grace: f32,
    pub(crate) burst: Pick,
    pub(crate) on: Pick,
    pub(crate) from: &'static Source,
    pub(crate) to: &'static Source,
    pub(crate) lift: f32,
    /// The chance (0 to 1) that a section start that lifts gets its burst.
    pub(crate) chance: f32,
}

/// What the director sees of the next section (see [`DirectorBinds::ahead`] and
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
        let seconds = cadence.triple("seconds", [24.0, 16.0, 10.0])?;
        cadence.require(
            seconds.iter().all(|s| *s > 0.0),
            "`when.seconds` must be above 0",
        )?;
        let gap = cadence.num("gap", 6.0)?;
        cadence.require(gap >= 0.0, "`when.gap` must not be negative")?;
        let lead = cadence.num("lead", 4.0)?;
        cadence.require(lead >= 0.0, "`when.lead` must not be negative")?;
        let (below, above) = (cadence.num("below", 0.35)?, cadence.num("above", 0.71)?);
        cadence.require(
            below <= above,
            "`when.below` must not be above `when.above`",
        )?;
        let sizes = one("director.sizes");
        let sizes = [
            sizes.wishes("quiet", [3.0, 3.0, 1.0, 0.0, 1.0])?,
            sizes.wishes("normal", [2.0, 3.0, 2.0, 1.0, 1.0])?,
            sizes.wishes("high", [1.0, 2.0, 3.0, 2.0, 1.0])?,
        ];
        let cut = one("director.cut");
        let grace = cut.num("grace", 2.0)?;
        cut.require(grace >= 0.0, "`when.grace` must not be negative")?;
        let burst = one("director.burst");
        let chance = burst.num("chance", 1.0)?;
        burst.require(
            (0.0..=1.0).contains(&chance),
            "`when.chance` must be from 0 to 1",
        )?;
        Ok(Self {
            cadence: cadence.pick(),
            below,
            above,
            seconds,
            gap,
            lead,
            sizes,
            cut: cut.pick(),
            grace,
            burst: burst.pick(),
            on: Pick::plain(burst.source_in("on", "beats", true)?),
            from: burst.source_in("from", "section_energy", false)?,
            to: burst.source_in("to", "next_energy", false)?,
            lift: burst.num("lift", 0.1)?,
            chance,
        })
    }

    /// How lively the section is: 0 quiet, 1 normal, 2 high. Normal where the cadence's source
    /// says nothing.
    pub fn tier(&self, music: &MusicValues) -> usize {
        let Some(energy) = self.cadence.said(music).reading() else {
            return 1;
        };
        self.tier_at(energy.value)
    }

    /// The tier of a section of this level (`section_level`, the cadence's source in
    /// `runtime.toml`): 0 quiet, 1 normal, 2 high. It is how the director reads the sections
    /// still to come, which no frame has said yet.
    pub fn tier_at(&self, level: f32) -> usize {
        if level < self.below {
            0
        } else if level > self.above {
            2
        } else {
            1
        }
    }

    /// Seconds between two cadence cuts in a tier, beat or not.
    pub fn seconds(&self, tier: usize) -> f32 {
        self.seconds[tier]
    }

    /// How much a shot of the size with this index (wide 0, medium, close, detail, top 4) is
    /// wanted in a section of a tier: `director.sizes`.
    pub fn size_wish(&self, tier: usize, size: usize) -> f32 {
        self.sizes[tier][size]
    }

    /// The shortest time in seconds between two cuts that are not inside one burst.
    pub fn gap(&self) -> f32 {
        self.gap
    }

    /// The chance (0 to 1) that a section start that lifts gets its burst of cuts: `director.burst`'s
    /// `chance`, 1 where the file leaves it out.
    pub fn burst_chance(&self) -> f32 {
        self.chance
    }

    /// Seconds before a known section start within which a cadence cut waits for it.
    pub fn lead(&self) -> f32 {
        self.lead
    }

    /// What a cut lands on in this frame: its source, or the fallback's, or nothing.
    pub fn cut(&self, music: &MusicValues) -> Said {
        self.cut.said(music)
    }

    /// What the source of `director.burst` says of the next section, if it says anything: seconds
    /// to it (-1 when unknown) and whether it is louder by the lift.
    fn looking_ahead(&self, music: &MusicValues) -> Option<Ahead> {
        let cue_in = self.burst.said(music).reading()?.value;
        Some(Ahead {
            cue_in,
            lifts: self.to.read(music).value - self.from.read(music).value >= self.lift,
        })
    }

    /// The next section start, if it is known (at 0 seconds or more). On any frame, beat or not:
    /// the cadence reads it to make way for the section start and for the burst before it.
    pub fn ahead(&self, music: &MusicValues) -> Option<Ahead> {
        self.looking_ahead(music)
            .filter(|ahead| ahead.cue_in >= 0.0)
    }

    /// The burst before a lift, if this frame is a beat it can cut on: the source of
    /// `director.burst` says something, `on` has an event, and the tempo is known.
    pub fn burst(&self, music: &MusicValues) -> Option<Ahead> {
        if music.tempo <= 0.0 || self.on.said(music).count() == 0 {
            return None;
        }
        self.looking_ahead(music)
    }
}

/// `look.change`: when a scene's look changes and over how long (see `scene::looks`).
#[derive(Clone, Debug, PartialEq)]
pub struct LookBinds {
    pub(crate) change: Pick,
    /// The least strength of a cue that changes the look where the song is charted (the
    /// sections have labels), and the least seconds between two changes.
    pub(crate) strength: f32,
    pub(crate) gap: f32,
    /// The same without a chart, where weaker cues change the look, further apart.
    pub(crate) strength_unlabelled: f32,
    pub(crate) gap_unlabelled: f32,
    /// How long the fade is, in beats while the beat is followed, else in seconds.
    pub(crate) fade: f32,
    pub(crate) fade_seconds: f32,
}

/// What changes a look in a frame (see [`LookBinds::cue`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LookCue {
    /// A section starts, and its label picks the look.
    Section,
    /// A weaker cue, with no sections to go by: the next look in the scene's order.
    Unlabelled,
}

impl LookBinds {
    fn new(binds: &[&Bound]) -> Result<Self> {
        let change = binds
            .iter()
            .find(|b| b.spec.target == "look.change")
            .expect("checked");
        let seconds = |key: &str, default: f32| -> Result<f32> {
            let value = change.num(key, default)?;
            change.require(value >= 0.0, &format!("`when.{key}` must not be negative"))?;
            Ok(value)
        };
        let strength = seconds("strength", 0.5)?;
        let strength_unlabelled = seconds("strength_unlabelled", 0.0)?;
        let gap = seconds("gap", 8.0)?;
        let gap_unlabelled = seconds("gap_unlabelled", 30.0)?;
        let fade = seconds("fade", 1.0)?;
        let fade_seconds = seconds("fade_seconds", 0.5)?;
        change.require(
            gap_unlabelled >= gap,
            "`when.gap_unlabelled` must not be less than `when.gap`",
        )?;
        Ok(Self {
            change: change.pick(),
            strength,
            gap,
            strength_unlabelled,
            gap_unlabelled,
            fade,
            fade_seconds,
        })
    }

    /// What changes the look in this frame, if anything: a cue of the strength the frame's level
    /// asks for. With a chart that is a section start; without one, any cue that is strong
    /// enough for `strength_unlabelled`.
    pub fn cue(&self, music: &MusicValues) -> Option<LookCue> {
        let reading = self
            .change
            .said(music)
            .reading()
            .filter(|reading| reading.count > 0)?;
        if Level::of(music) == Level::C {
            (reading.value >= self.strength_unlabelled).then_some(LookCue::Unlabelled)
        } else {
            (reading.value >= self.strength).then_some(LookCue::Section)
        }
    }

    /// The least seconds between two changes after a change of this kind.
    pub fn gap(&self, cue: LookCue) -> f32 {
        match cue {
            LookCue::Section => self.gap,
            LookCue::Unlabelled => self.gap_unlabelled,
        }
    }

    /// How long the fade to another look lasts in this frame's music, in seconds: `fade` beats
    /// while the beat is followed, else `fade_seconds`.
    pub fn fade_length(&self, music: &MusicValues) -> f32 {
        if music.locked > FOLLOWED && music.tempo > 0.0 {
            self.fade * 60.0 / music.tempo
        } else {
            self.fade_seconds
        }
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
    /// The bind as written: the `when` as an inline table and the `fallback` as a TOML value.
    when: String,
    fallback: String,
}

/// The runtime's decisions, read from a file of `[[bind]]`s.
#[derive(Clone, Debug)]
pub struct RuntimeBindings {
    pub moments: Arc<MomentBinds>,
    pub director: Arc<DirectorBinds>,
    pub looks: Arc<LookBinds>,
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
            looks: Arc::new(LookBinds::new(&area("look."))?),
            course: CourseBinds::new(&area("course."))?,
            rows: bound
                .iter()
                .map(|b| Row {
                    target: b.spec.target,
                    how: b.how.clone(),
                    pick: b.pick,
                    when: declared::when_text(&b.when),
                    fallback: declared::fallback_text(b.fallback.as_ref()),
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
                        when: row.when.clone(),
                        fallback: row.fallback.clone(),
                        ease: String::new(),
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
                    when: row.when.clone(),
                    fallback: row.fallback.clone(),
                    ease: String::new(),
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

impl Default for LookBinds {
    fn default() -> Self {
        (*RuntimeBindings::embedded().looks).clone()
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
    fn the_chance_of_a_burst_is_from_0_to_1() {
        assert!(error_of("chance = 0.5", "chance = 1.5").contains("`when.chance`"));
        assert!(error_of("chance = 0.5", "chance = -0.1").contains("`when.chance`"));
        let left_out = embedded_with(", chance = 0.5", "");
        assert_eq!(left_out.director.burst_chance(), 1.0);
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
        assert_eq!(count("look.change"), 1);
    }

    #[test]
    fn the_sizes_are_read_per_tier_and_a_bad_table_is_named() {
        let sizes = RuntimeBindings::embedded().director.sizes;
        assert_eq!(sizes[0], [3.0, 3.0, 1.0, 0.0, 1.0]);
        assert_eq!(sizes[1], [2.0, 3.0, 2.0, 1.0, 1.0]);
        assert_eq!(sizes[2], [1.0, 2.0, 3.0, 2.0, 1.0]);
        assert_eq!(RuntimeBindings::embedded().director.size_wish(2, 3), 2.0);
        let bad = |to: &str, expect: &str| {
            let message = error_of("quiet = [3, 3, 1, 0, 1]", to);
            assert!(
                message.contains("director.sizes") && message.contains(expect),
                "{message}"
            );
        };
        bad("quiet = [3, 3, 1, 0]", "five numbers");
        bad("quiet = [3, 3, 1, 0, 1, 1]", "five numbers");
        bad("quiet = 3", "five numbers");
        bad("quiet = [3, 3, -1, 0, 1]", "negative");
        bad("quiet = [0, 0, 0, 0, 0]", "all 0");
        bad("quiet = [3, 3, \"x\", 0, 1]", "number");
        bad("quiets = [3, 3, 1, 0, 1]", "quiets");
    }

    #[test]
    fn look_change_reads_cues_by_the_level_and_a_bad_table_is_named() {
        let binds = RuntimeBindings::embedded().looks.clone();
        let charted = |strength: f32| MusicValues {
            has_chart: 1,
            cues: 1,
            cue: strength,
            ..MusicValues::default()
        };
        // With a chart only a section start (a cue of 0.5 or more) changes the look.
        assert_eq!(binds.cue(&charted(0.5)), Some(LookCue::Section));
        assert_eq!(binds.cue(&charted(0.49)), None);
        assert_eq!(
            binds.cue(&MusicValues {
                has_chart: 1,
                ..MusicValues::default()
            }),
            None
        );
        // Without, any cue does, further apart.
        let weak = MusicValues {
            cues: 1,
            cue: 0.1,
            ..MusicValues::default()
        };
        assert_eq!(binds.cue(&weak), Some(LookCue::Unlabelled));
        assert_eq!(
            (binds.gap(LookCue::Section), binds.gap(LookCue::Unlabelled)),
            (8.0, 30.0)
        );
        // The fade is a beat while the beat is followed, else half a second.
        let beat = MusicValues {
            has_chart: 1,
            locked: 1.0,
            tempo: 60.0,
            ..MusicValues::default()
        };
        assert_eq!(binds.fade_length(&beat), 1.0);
        assert_eq!(binds.fade_length(&MusicValues::default()), 0.5);
        let bad = |from: &str, to: &str, expect: &str| {
            let message = error_of(from, to);
            assert!(
                message.contains("look.change") && message.contains(expect),
                "{message}"
            );
        };
        bad("gap = 8.0", "gap = -1.0", "gap");
        bad("fade = 1.0", "fade = \"long\"", "fade");
        bad("fade_seconds = 0.5", "fade_second = 0.5", "fade_second");
        bad("gap = 8.0", "gap = 40.0", "gap_unlabelled");
        bad(
            "how = \"trigger\"\nwhen = { strength = 0.5, gap",
            "how = \"follow\"\nwhen = { strength = 0.5, gap",
            "how",
        );
    }

    #[test]
    fn a_mistake_names_the_bind() {
        let cadence = "target = \"director.cadence\"";
        let message = error_of(cadence, "target = \"director.cadense\"");
        assert!(
            message.contains("director.cadense") && message.contains("targets are"),
            "{message}"
        );
        let message = error_of("source = \"section_level\"", "source = \"loudness\"");
        assert!(
            message.contains("director.cadence") && message.contains("loudness"),
            "{message}"
        );
        let message = error_of("when = { strength = 0.8 }", "when = { strenght = 0.8 }");
        assert!(
            message.contains("moments.strong") && message.contains("strenght"),
            "{message}"
        );
        let message = error_of(
            "source = \"section_level\"\nhow = \"follow\"",
            "source = \"section_level\"\nhow = \"follow\"\nfallback = \"hold\"",
        );
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
        // The section's level means something at every level.
        for level in [Level::A, Level::B, Level::C] {
            assert!(matches!(cadence.said(&at(level)), Said::Source(_)));
        }
        // The downbeats only with a followed beat, else the fallback.
        let cut = RuntimeBindings::embedded().director.cut;
        assert!(matches!(cut.said(&at(Level::A)), Said::Source(_)));
        assert!(matches!(
            cut.said(&at(Level::C)),
            Said::Fallback(source, _) if source.name == "hits"
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
            level: 0.8,
            ..MusicValues::default()
        };
        let report = runtime.report(&music);
        let cadence = report
            .iter()
            .find(|r| r.target == "director.cadence")
            .unwrap();
        assert_eq!(cadence.level, Level::C);
        assert_eq!(cadence.used, Used::Source);
        assert_eq!(cadence.output, 0.8);
        assert_eq!(cadence.source, "section_level");
        let cut = report.iter().find(|r| r.target == "director.cut").unwrap();
        assert_eq!(cut.used, Used::Fallback("hits"), "no downbeats at level C");
        let burst = report
            .iter()
            .find(|r| r.target == "director.burst")
            .unwrap();
        assert_eq!(burst.used, Used::Rest, "off at level C");
        let gap = report.iter().find(|r| r.target == "moments.gap").unwrap();
        assert_eq!(gap.used, Used::Value);
        assert_eq!(gap.output, 2.0);
    }

    #[test]
    fn the_report_carries_each_bind_as_written() {
        let report = RuntimeBindings::embedded().report(&MusicValues::default());
        let row = |target: &str| report.iter().find(|r| r.target == target).unwrap();
        assert_eq!(row("moments.gap").when, "{ every = 8 }");
        assert_eq!(row("moments.gap").fallback, "{ value = 2.0 }");
        assert_eq!(row("director.cut").fallback, "\"hits\"");
        assert_eq!(row("director.cadence").fallback, "");
        assert_eq!(
            row("director.cadence").when,
            "{ above = 0.71, below = 0.35, gap = 6.0, lead = 4.0, seconds = [24.0, 16.0, 10.0] }"
        );
    }
}
