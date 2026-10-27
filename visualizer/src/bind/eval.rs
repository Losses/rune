//! One evaluator per `[[bind]]`: it owns the binding's state, is ticked every frame with the
//! frame's time step and music, and says the value of the input it drives and which source
//! produced it (see "Bindings" in `docs/music.md`).
//!
//! A binding names a source and a `how`. When the source means nothing at the frame's level (see
//! [`Source::meaningful_at`]) the binding's `fallback` takes over: another source read the same
//! way, a fixed value, the last output, or the input's rest. A binding with no fallback uses its
//! source at every level, as a scene that reads the raw signal does.

use anyhow::{Context, Result, bail, ensure};

use super::declared;
use super::level::Level;
use super::sources::{self, Reading, Source};
use crate::course;
use crate::scene::{Bind, FallbackDecl, InputDecl, MusicValues};
use crate::switching::pick_by_label;

/// The `how`s a binding knows.
pub const HOWS: &[&str] = &["follow", "trigger", "step", "pick", "accumulate"];
/// The most options a `pick` has.
pub const MAX_OPTIONS: u32 = 64;

/// What a source is mapped through, from 0..1 to 0..1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Curve {
    Linear,
    /// A smoothstep: gentle at both ends.
    Smooth,
    /// Slow at first, then fast (squared).
    In,
    /// Fast at first, then slow.
    Out,
}

impl Curve {
    const NAMES: &'static str = "linear, smooth, in, out";

    fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "linear" => Self::Linear,
            "smooth" => Self::Smooth,
            "in" => Self::In,
            "out" => Self::Out,
            _ => return None,
        })
    }

    /// The curve at `t` (clamped to 0..1).
    pub fn at(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::Linear => t,
            Self::Smooth => t * t * (3.0 - 2.0 * t),
            Self::In => t * t,
            Self::Out => 1.0 - (1.0 - t) * (1.0 - t),
        }
    }
}

/// Whether a beat time is eased: `floor(t) + ease(fract(t))`, continuous but with a push at the
/// start of every beat (see [`course::eased`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ease {
    None,
    Out,
}

impl Ease {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "none" => Some(Self::None),
            "out" => Some(Self::Out),
            _ => None,
        }
    }

    /// The beat time `t` as this ease makes it.
    fn apply(self, t: f32) -> f32 {
        match self {
            Self::None => t,
            Self::Out => course::eased(t),
        }
    }
}

/// What an `accumulate` counts its rate per.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Per {
    Beat,
    Second,
}

/// A binding's `how` with its parameters.
#[derive(Clone, Debug, PartialEq)]
enum Mode {
    Follow {
        from: [f32; 2],
        to: [f32; 2],
        curve: Curve,
        /// A time constant in beats; 0 for none.
        smooth: f32,
        ease: Ease,
        clamp: bool,
    },
    Trigger {
        threshold: f32,
        /// In beats.
        attack: f32,
        release: f32,
        scale: bool,
    },
    Step {
        every: f32,
        ease: Ease,
        wrap: Option<f32>,
    },
    Pick {
        /// The indices 0..options, for [`pick_by_label`].
        options: Vec<u32>,
    },
    Accumulate {
        rate: f32,
        per: Per,
        wrap: Option<f32>,
    },
}

/// What a binding does where its source means nothing.
#[derive(Clone, Copy)]
enum Fallback {
    /// Nothing declared: the source is used all the same.
    None,
    Source(&'static Source),
    Value(f32),
    Hold,
    Rest,
}

/// Which of a binding's means produced its output in a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Used {
    /// Its own source.
    Source,
    /// The fallback source of this name.
    Fallback(&'static str),
    /// The fallback's fixed value.
    Value,
    /// The last output, held.
    Hold,
    /// The input's rest.
    Rest,
}

/// What an evaluator said in its last frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tick {
    pub level: Level,
    pub used: Used,
    /// The value of the binding's own source (whichever means produced the output).
    pub source: f32,
    pub output: f32,
}

impl Default for Tick {
    fn default() -> Self {
        Self {
            level: Level::default(),
            used: Used::Source,
            source: 0.0,
            output: 0.0,
        }
    }
}

/// What a frame's output comes from.
enum Means {
    Source(&'static Source, Reading),
    Fallback(&'static Source, Reading),
    Value(f32),
    Hold,
    Rest,
}

/// The parts of the state the `how`s keep, as they need them.
#[derive(Clone, Debug, Default)]
struct State {
    /// A smoothed value (`follow`), or the envelope (`trigger`).
    level: Option<f32>,
    /// The envelope's peak and whether it is still rising to it.
    peak: f32,
    rising: bool,
    /// The source's last value (`pick`, to change only when it changes) and the index chosen.
    seen: Option<f32>,
    index: u32,
    /// The running total (`accumulate`).
    total: f32,
}

/// The evaluator of one `[[bind]]`.
pub struct Evaluator {
    target: String,
    how: &'static str,
    source: &'static Source,
    fallback: Fallback,
    rest: f32,
    /// The top of the input's range, where a `trigger` peaks.
    top: f32,
    mode: Mode,
    state: State,
    last: f32,
    tick: Tick,
    /// The bind as it was written, for a lab.
    declared: Declared,
    /// What it was made from, to tell whether a new bind is the same one.
    bind: Bind,
    input: InputDecl,
}

/// The parts of a bind a lab shows as written (see [`super::declared`]).
#[derive(Clone, Debug, Default, PartialEq)]
struct Declared {
    when: String,
    fallback: String,
    ease: String,
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

/// The parameters of a bind: `when`, read key by key, so that a key that nothing reads is named.
struct When<'a> {
    table: &'a toml::Table,
    read: Vec<&'static str>,
}

impl<'a> When<'a> {
    fn number(&mut self, key: &'static str, default: f32) -> Result<f32> {
        self.read.push(key);
        match self.table.get(key) {
            Some(value) => number(value, &format!("`when.{key}`")),
            None => Ok(default),
        }
    }

    fn optional(&mut self, key: &'static str) -> Result<Option<f32>> {
        self.read.push(key);
        self.table
            .get(key)
            .map(|value| number(value, &format!("`when.{key}`")))
            .transpose()
    }

    fn pair(&mut self, key: &'static str) -> Result<Option<[f32; 2]>> {
        self.read.push(key);
        let Some(value) = self.table.get(key) else {
            return Ok(None);
        };
        let what = format!("`when.{key}`");
        let toml::Value::Array(items) = value else {
            bail!("{what} must be [lo, hi]");
        };
        ensure!(items.len() == 2, "{what} must be [lo, hi]");
        Ok(Some([number(&items[0], &what)?, number(&items[1], &what)?]))
    }

    fn flag(&mut self, key: &'static str, default: bool) -> Result<bool> {
        self.read.push(key);
        match self.table.get(key) {
            None => Ok(default),
            Some(toml::Value::Boolean(v)) => Ok(*v),
            Some(other) => bail!("`when.{key}` must be true or false, not {other}"),
        }
    }

    fn word(&mut self, key: &'static str) -> Result<Option<&'a str>> {
        self.read.push(key);
        match self.table.get(key) {
            None => Ok(None),
            Some(toml::Value::String(v)) => Ok(Some(v)),
            Some(other) => bail!("`when.{key}` must be a word, not {other}"),
        }
    }

    /// Fails if the table has a key that was not read.
    fn finish(self, how: &str) -> Result<()> {
        for key in self.table.keys() {
            ensure!(
                self.read.contains(&key.as_str()),
                "unknown key `{key}` in `when` of a {how} bind (it takes: {})",
                self.read.join(", ")
            );
        }
        Ok(())
    }
}

impl Evaluator {
    /// The evaluator of `bind`, which drives `input`. Fails, naming what is wrong, if the bind
    /// names a source or a `how` that does not exist, a `when` key its `how` does not take, or a
    /// value that makes no sense.
    pub fn new(bind: &Bind, input: &InputDecl) -> Result<Self> {
        Self::compile(bind, input).with_context(|| format!("[[bind]] for `{}`", bind.target))
    }

    fn compile(bind: &Bind, input: &InputDecl) -> Result<Self> {
        let source = sources::find(&bind.source).with_context(|| {
            format!(
                "no source `{}` (the sources are: {})",
                bind.source,
                sources::names()
            )
        })?;
        let how = HOWS
            .iter()
            .copied()
            .find(|how| *how == bind.how)
            .with_context(|| {
                format!("no how `{}` (it is one of: {})", bind.how, HOWS.join(", "))
            })?;
        if how == "trigger" {
            ensure!(
                source.event,
                "a trigger needs a source that counts events (hits, bars, beats, cues, impacts, notes, moment.strong, moment.medium), not `{}`",
                source.name
            );
        }
        let ease = bind
            .ease
            .as_deref()
            .map(|name| {
                Ease::parse(name)
                    .with_context(|| format!("no ease `{name}` (it is one of: none, out)"))
            })
            .transpose()?;
        ensure!(
            ease.is_none() || how == "follow" || how == "step",
            "`ease` is for follow and step, not {how}"
        );
        let [lo, hi] = input.range;
        let mut when = When {
            table: &bind.when,
            read: Vec::new(),
        };
        let mode = match how {
            "follow" => {
                let from = when.pair("from")?.unwrap_or([0.0, 1.0]);
                ensure!(
                    from[0] != from[1],
                    "`when.from` must have two different ends"
                );
                let to = when.pair("to")?.unwrap_or([lo, hi]);
                let curve = match when.word("curve")? {
                    None => Curve::Linear,
                    Some(name) => Curve::parse(name).with_context(|| {
                        format!("no curve `{name}` (it is one of: {})", Curve::NAMES)
                    })?,
                };
                let smooth = when.number("smooth", 0.0)?;
                ensure!(smooth >= 0.0, "`when.smooth` must not be negative");
                let clamp = when.flag("clamp", true)?;
                Mode::Follow {
                    from,
                    to,
                    curve,
                    smooth,
                    ease: ease.unwrap_or(Ease::None),
                    clamp,
                }
            }
            "trigger" => {
                let threshold = when.number("strength", 0.0)?;
                let attack = when.number("attack", 0.0)?;
                let release = when.number("release", 0.25)?;
                ensure!(
                    attack >= 0.0 && release >= 0.0,
                    "`when.attack` and `when.release` must not be negative"
                );
                Mode::Trigger {
                    threshold,
                    attack,
                    release,
                    scale: when.flag("scale", true)?,
                }
            }
            "step" => {
                let every = when.number("every", 1.0)?;
                ensure!(every > 0.0, "`when.every` must be above 0");
                Mode::Step {
                    every,
                    ease: ease.unwrap_or(Ease::Out),
                    wrap: wrap_of(&mut when)?,
                }
            }
            "pick" => {
                let options = when.number("options", 0.0)?;
                ensure!(
                    options >= 1.0 && options <= MAX_OPTIONS as f32 && options.fract() == 0.0,
                    "`when.options` must be a whole number from 1 to {MAX_OPTIONS}"
                );
                Mode::Pick {
                    options: (0..options as u32).collect(),
                }
            }
            _ => {
                let rate = when.number("rate", 1.0)?;
                let per = match when.word("per")? {
                    None | Some("beat") => Per::Beat,
                    Some("second") => Per::Second,
                    Some(other) => bail!("no `when.per` `{other}` (it is beat or second)"),
                };
                Mode::Accumulate {
                    rate,
                    per,
                    wrap: wrap_of(&mut when)?,
                }
            }
        };
        when.finish(how)?;
        let fallback = match &bind.fallback {
            None => Fallback::None,
            Some(FallbackDecl::Value { value }) => {
                ensure!(
                    value.is_finite(),
                    "`fallback` value must be a finite number"
                );
                Fallback::Value(*value)
            }
            Some(FallbackDecl::Word(word)) => match word.as_str() {
                "hold" => Fallback::Hold,
                "rest" => Fallback::Rest,
                name => {
                    let fallback = sources::find(name).with_context(|| {
                        format!(
                            "no fallback `{name}` (a source, `hold`, `rest` or {{ value = x }}; the sources are: {})",
                            sources::names()
                        )
                    })?;
                    ensure!(
                        how != "trigger" || fallback.event,
                        "a trigger needs a fallback source that counts events, not `{name}`"
                    );
                    Fallback::Source(fallback)
                }
            },
        };
        let rest = input.rest();
        Ok(Self {
            target: bind.target.clone(),
            how,
            source,
            fallback,
            rest,
            top: hi,
            mode,
            state: State {
                total: rest,
                ..State::default()
            },
            last: rest,
            tick: Tick {
                output: rest,
                ..Tick::default()
            },
            declared: Declared {
                when: declared::when_text(&bind.when),
                fallback: declared::fallback_text(bind.fallback.as_ref()),
                ease: bind.ease.clone().unwrap_or_default(),
            },
            bind: bind.clone(),
            input: input.clone(),
        })
    }

    pub fn target(&self) -> &str {
        &self.target
    }

    pub fn source(&self) -> &'static str {
        self.source.name
    }

    pub fn how(&self) -> &'static str {
        self.how
    }

    /// What it said in its last frame.
    pub fn last(&self) -> Tick {
        self.tick
    }

    /// Advance by a frame of `dt` seconds in `music`, which is at `level`, and say the output.
    pub fn tick(&mut self, dt: f32, music: &MusicValues, level: Level) -> Tick {
        let own = self.source.read(music);
        let means = match self.fallback {
            Fallback::None => Means::Source(self.source, own),
            _ if self.source.meaningful_at(level) => Means::Source(self.source, own),
            Fallback::Source(other) => Means::Fallback(other, other.read(music)),
            Fallback::Value(value) => Means::Value(value),
            Fallback::Hold => Means::Hold,
            Fallback::Rest => Means::Rest,
        };
        let (used, output) = match means {
            Means::Source(source, reading) => {
                (Used::Source, self.advance(dt, music, source, reading))
            }
            Means::Fallback(source, reading) => (
                Used::Fallback(source.name),
                self.advance(dt, music, source, reading),
            ),
            Means::Value(value) => (Used::Value, value),
            Means::Hold => (Used::Hold, self.last),
            Means::Rest => (Used::Rest, self.rest),
        };
        // A smoothed follow glides on from where the fallback put it, not from where it was.
        if matches!(used, Used::Value | Used::Rest) && self.state.level.is_some() {
            self.state.level = Some(output);
        }
        self.last = output;
        self.tick = Tick {
            level,
            used,
            source: own.value,
            output,
        };
        self.tick
    }

    /// Move the state on a frame with what `source` said, and give the output.
    fn advance(&mut self, dt: f32, music: &MusicValues, source: &Source, reading: Reading) -> f32 {
        let beats = beats_in(dt, music);
        match &self.mode {
            Mode::Follow {
                from,
                to,
                curve,
                smooth,
                ease,
                clamp,
            } => {
                let x = ease.apply(reading.value);
                let t = (x - from[0]) / (from[1] - from[0]);
                // Beyond the ends the map goes straight on unless it is clamped.
                let shaped = if *clamp {
                    curve.at(t)
                } else {
                    curve.at(t) + (t - t.clamp(0.0, 1.0))
                };
                let target = to[0] + shaped * (to[1] - to[0]);
                let level = match (self.state.level, *smooth > 0.0) {
                    (Some(level), true) => {
                        level + (target - level) * (1.0 - (-beats / smooth).exp())
                    }
                    _ => target,
                };
                self.state.level = Some(level);
                level
            }
            Mode::Trigger {
                threshold,
                attack,
                release,
                scale,
            } => {
                let level = self.state.level.unwrap_or(0.0);
                let level = if reading.count > 0 && reading.value >= *threshold {
                    self.state.peak = if *scale {
                        reading.value.clamp(0.0, 1.0)
                    } else {
                        1.0
                    };
                    self.state.rising = *attack > 0.0;
                    if self.state.rising {
                        level
                    } else {
                        self.state.peak
                    }
                } else if self.state.rising {
                    let level = (level + self.state.peak * beats / attack).min(self.state.peak);
                    self.state.rising = level < self.state.peak;
                    level
                } else if *release > 0.0 {
                    (level - self.state.peak * beats / release).max(0.0)
                } else {
                    0.0
                };
                self.state.level = Some(level);
                self.rest + level * (self.top - self.rest)
            }
            Mode::Step { every, ease, wrap } => {
                let t = reading.value.max(0.0) / every;
                let counted = match ease {
                    Ease::None => t.floor(),
                    Ease::Out => course::eased(t),
                };
                wrap.map_or(counted, |wrap| counted.rem_euclid(wrap))
            }
            Mode::Pick { options } => {
                if source.event {
                    self.state.index =
                        (self.state.index + reading.count) % options.len().max(1) as u32;
                } else if self.state.seen != Some(reading.value) {
                    self.state.seen = Some(reading.value);
                    self.state.index = pick_by_label(options, reading.value).unwrap_or_default();
                }
                self.state.index as f32
            }
            Mode::Accumulate { rate, per, wrap } => {
                let by = match per {
                    Per::Beat => dt * beats_per_second(music),
                    Per::Second => dt,
                };
                let total = self.state.total + reading.value * rate * by;
                self.state.total = wrap.map_or(total, |wrap| total.rem_euclid(wrap));
                self.state.total
            }
        }
    }
}

fn wrap_of(when: &mut When) -> Result<Option<f32>> {
    let wrap = when.optional("wrap")?;
    ensure!(wrap.is_none_or(|w| w > 0.0), "`when.wrap` must be above 0");
    Ok(wrap)
}

/// How many beats go by in a second: the tempo while the beat is followed, else 1 (seconds are
/// all there is).
fn beats_per_second(music: &MusicValues) -> f32 {
    if course::followed(music) {
        music.tempo / 60.0
    } else {
        1.0
    }
}

/// `dt` seconds as beats, for the times a bind gives in beats: beats of the tempo while the beat
/// is followed, else of [`course::UNFOLLOWED_BEAT_SECONDS`].
fn beats_in(dt: f32, music: &MusicValues) -> f32 {
    if course::followed(music) {
        dt * music.tempo / 60.0
    } else {
        dt / course::UNFOLLOWED_BEAT_SECONDS
    }
}

/// All the bindings of a scene with the values of its inputs: what a scene keeps to drive
/// `struct Inputs`.
pub struct Bindings {
    /// The bindings, each with the place of its input.
    evaluators: Vec<(usize, Evaluator)>,
    /// The value of each input, in the manifest's order; an input nothing drives stays at rest.
    values: Vec<f32>,
}

/// What a lab reads of one binding.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub target: String,
    pub source: &'static str,
    pub how: &'static str,
    pub tick: Tick,
    /// The bind as it was written: the `when` as an inline table, the `fallback` as a TOML value
    /// (empty for none) and the `ease` (empty for none).
    pub when: String,
    pub fallback: String,
    pub ease: String,
}

impl Bindings {
    /// The evaluators of `binds`, which drive `inputs`. Fails on a bind of no input or one that
    /// does not compile (see [`Evaluator::new`]).
    pub fn new(inputs: &[InputDecl], binds: &[Bind]) -> Result<Self> {
        let evaluators = binds
            .iter()
            .map(|bind| {
                let slot = inputs
                    .iter()
                    .position(|input| input.id == bind.target)
                    .with_context(|| {
                        format!(
                            "[[bind]] for `{}`: no input of that name (declare it with [[input]])",
                            bind.target
                        )
                    })?;
                Ok((slot, Evaluator::new(bind, &inputs[slot])?))
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            evaluators,
            values: inputs.iter().map(InputDecl::rest).collect(),
        })
    }

    /// The bindings of `binds` for `inputs`, where a binding that is the same as one of these
    /// (the same bind of the same input) takes over its evaluator and so its state (an envelope,
    /// an accumulated distance) instead of starting over. Fails as [`Bindings::new`] does, and
    /// then these are left as they were.
    pub fn renewed(&mut self, inputs: &[InputDecl], binds: &[Bind]) -> Result<Self> {
        let mut next = Self::new(inputs, binds)?;
        for (slot, fresh) in &mut next.evaluators {
            let same = self
                .evaluators
                .iter_mut()
                .find(|(_, old)| old.bind == fresh.bind && old.input == fresh.input);
            if let Some((_, old)) = same {
                std::mem::swap(fresh, old);
                next.values[*slot] = fresh.tick.output;
            }
        }
        Ok(next)
    }

    /// Advance every binding by a frame and give the values of the inputs.
    pub fn tick(&mut self, dt: f32, music: &MusicValues) -> &[f32] {
        let level = Level::of(music);
        for (slot, evaluator) in &mut self.evaluators {
            self.values[*slot] = evaluator.tick(dt, music, level).output;
        }
        &self.values
    }

    /// The values of the inputs as of the last frame (their rests before the first).
    pub fn values(&self) -> &[f32] {
        &self.values
    }

    /// What each binding said in the last frame.
    pub fn report(&self) -> Vec<Report> {
        self.evaluators
            .iter()
            .map(|(_, evaluator)| Report {
                target: evaluator.target.clone(),
                source: evaluator.source.name,
                how: evaluator.how,
                tick: evaluator.tick,
                when: evaluator.declared.when.clone(),
                fallback: evaluator.declared.fallback.clone(),
                ease: evaluator.declared.ease.clone(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A chart with the beat followed, at 120 bpm, so that a beat is half a second.
    fn a() -> MusicValues {
        MusicValues {
            has_chart: 1,
            locked: 1.0,
            tempo: 120.0,
            ..MusicValues::default()
        }
    }

    fn at(level: Level) -> MusicValues {
        match level {
            Level::A => a(),
            Level::B => MusicValues { locked: 0.0, ..a() },
            Level::C => MusicValues {
                has_chart: 0,
                locked: 0.0,
                ..a()
            },
        }
    }

    fn input(range: [f32; 2], rest: Option<f32>) -> InputDecl {
        InputDecl {
            id: "x".into(),
            label: None,
            range,
            rest,
        }
    }

    fn bind(text: &str) -> Bind {
        toml::from_str(&format!("target = \"x\"\n{text}")).unwrap()
    }

    fn evaluator(text: &str) -> Evaluator {
        Evaluator::new(&bind(text), &input([0.0, 1.0], None)).unwrap()
    }

    fn error(text: &str) -> String {
        match Evaluator::new(&bind(text), &input([0.0, 1.0], None)) {
            Ok(_) => panic!("{text} was accepted"),
            Err(e) => format!("{e:#}"),
        }
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    /// Ticks of 0.1 s at level A with the music of each step; the outputs.
    fn run(evaluator: &mut Evaluator, frames: &[MusicValues]) -> Vec<f32> {
        frames
            .iter()
            .map(|m| evaluator.tick(0.1, m, Level::of(m)).output)
            .collect()
    }

    #[test]
    fn follow_maps_a_range_through_a_curve() {
        let with = |m: MusicValues| MusicValues { drive: 0.5, ..m };
        let drive = |text: &str| {
            evaluator(&format!("source = \"drive\"\nhow = \"follow\"\n{text}"))
                .tick(0.1, &with(a()), Level::A)
                .output
        };
        // The input's range is the default target and 0..1 the default source range.
        assert!(near(drive(""), 0.5));
        assert!(near(drive("when = { from = [0.0, 2.0] }"), 0.25));
        assert!(near(drive("when = { to = [10, 20] }"), 15.0));
        assert!(near(drive("when = { curve = \"in\" }"), 0.25));
        assert!(near(drive("when = { curve = \"out\" }"), 0.75));
        assert!(near(drive("when = { curve = \"smooth\" }"), 0.5));
        // Outside the source range it holds the ends; a reversed target range reverses it.
        assert!(near(drive("when = { from = [0.6, 1.0] }"), 0.0));
        assert!(near(drive("when = { to = [1.0, 0.0] }"), 0.5));
    }

    #[test]
    fn follow_smooths_with_a_time_constant_in_beats() {
        let text = "source = \"drive\"\nhow = \"follow\"\nwhen = { smooth = 1.0 }";
        let with = |level, drive| MusicValues { drive, ..at(level) };
        // The first frame starts at the value, not gliding up from rest.
        let mut first = evaluator(text);
        assert_eq!(first.tick(0.1, &with(Level::A, 1.0), Level::A).output, 1.0);
        // A time constant of one beat is 0.5 s: after 0.5 s it is 1 - 1/e of the way, and
        // without a followed beat a beat counts as half a second too.
        for level in [Level::A, Level::C] {
            let mut e = evaluator(text);
            assert_eq!(e.tick(0.1, &with(level, 0.0), level).output, 0.0);
            let out: Vec<f32> = (0..5)
                .map(|_| e.tick(0.1, &with(level, 1.0), level).output)
                .collect();
            assert!(near(out[4], 1.0 - (-1.0f32).exp()), "{out:?}");
            assert!(out.windows(2).all(|w| w[1] > w[0]));
        }
    }

    #[test]
    fn follow_clock_with_ease_pushes_on_every_beat_and_clamp_off_goes_on() {
        let mut e = evaluator(
            "source = \"clock\"\nhow = \"follow\"\nease = \"out\"\nwhen = { from = [0, 8], to = [0, 8], clamp = false }",
        );
        let clock = |t: f32| MusicValues { clock: t, ..a() };
        let out = run(
            &mut e,
            &[clock(0.0), clock(0.2), clock(0.99), clock(1.0), clock(3.5)],
        );
        // Eased: most of a beat is done early, and the end of a beat is the start of the next.
        assert!(out[1] > 0.7 && out[2] > 0.99 && near(out[3], 1.0));
        // Unclamped, it runs past the end of the range.
        assert!(out[4] > 3.0);
        let mut clamped =
            evaluator("source = \"clock\"\nhow = \"follow\"\nwhen = { from = [0, 2] }");
        assert_eq!(run(&mut clamped, &[clock(5.0)])[0], 1.0);
    }

    fn hit(strength: f32) -> MusicValues {
        MusicValues {
            hits: 1,
            hit: strength,
            ..a()
        }
    }

    #[test]
    fn trigger_peaks_with_the_strength_and_releases_in_beats() {
        let mut e = evaluator(
            "source = \"hits\"\nhow = \"trigger\"\nwhen = { strength = 0.3, release = 1.0 }",
        );
        let quiet = MusicValues { hit: 0.0, ..a() };
        // Under the threshold nothing happens; at it the envelope jumps to the strength.
        assert_eq!(run(&mut e, &[hit(0.2)])[0], 0.0);
        assert!(near(run(&mut e, &[hit(0.8)])[0], 0.8));
        // A release of one beat (0.5 s at 120 bpm) falls from the peak to nothing: 0.1 s is a
        // fifth of it.
        let out = run(&mut e, &[quiet; 6]);
        assert!(near(out[0], 0.8 * 0.8));
        assert!(near(out[1], 0.8 * 0.6));
        assert_eq!(out[5], 0.0);
        // With `scale = false` every hit peaks at the top of the range.
        let mut flat = evaluator("source = \"hits\"\nhow = \"trigger\"\nwhen = { scale = false }");
        assert_eq!(run(&mut flat, &[hit(0.4)])[0], 1.0);
        // A frame with several hits is one trigger; a hit frame with no count is none.
        let mut many = evaluator("source = \"hits\"\nhow = \"trigger\"");
        assert!(near(
            run(
                &mut many,
                &[MusicValues {
                    hits: 3,
                    hit: 0.5,
                    ..a()
                }]
            )[0],
            0.5
        ));
        // The envelope then falls from 0.5 (the default release is a quarter beat).
        let after = MusicValues {
            hits: 0,
            hit: 0.5,
            ..a()
        };
        assert!(run(&mut many, &[after])[0] < 0.5);
    }

    #[test]
    fn trigger_rises_over_the_attack_and_peaks_at_the_top_of_the_range() {
        let decl = input([2.0, 6.0], Some(3.0));
        let mut e = Evaluator::new(
            &bind("source = \"hits\"\nhow = \"trigger\"\nwhen = { attack = 1.0, release = 1.0 }"),
            &decl,
        )
        .unwrap();
        let quiet = MusicValues { hit: 0.0, ..a() };
        // From the input's rest to the top of its range: 3 up to 6.
        assert_eq!(e.tick(0.1, &quiet, Level::A).output, 3.0);
        assert_eq!(e.tick(0.1, &hit(1.0), Level::A).output, 3.0);
        // A beat of attack is 0.5 s: 0.1 s is a fifth of the way, and it peaks at the end.
        let out = run(&mut e, &[quiet; 5]);
        assert!(near(out[0], 3.0 + 3.0 * 0.2));
        assert!(near(out[4], 6.0));
        // Then it falls back to rest.
        let out = run(&mut e, &[quiet; 6]);
        assert!(out[0] < 6.0 && near(out[5], 3.0));
    }

    #[test]
    fn step_counts_on_the_eased_clock() {
        let mut e = evaluator("source = \"clock\"\nhow = \"step\"");
        let clock = |t: f32| MusicValues { clock: t, ..a() };
        let out = run(
            &mut e,
            &[
                clock(0.0),
                clock(0.1),
                clock(0.5),
                clock(0.999),
                clock(1.0),
                clock(1.1),
            ],
        );
        // Continuous, rising, and at each whole beat it has counted one more, with a push at
        // the start of the next.
        assert_eq!(out[0], 0.0);
        assert!(out[1] > 0.5 && out[1] < out[2] && out[3] < 1.0001 && out[3] > 0.99);
        assert!(near(out[4], 1.0) && out[5] > 1.5);
        assert!(out.windows(2).all(|w| w[1] >= w[0]));
        // Every two beats; plain floors without an ease; a wrap.
        let mut two = evaluator("source = \"clock\"\nhow = \"step\"\nwhen = { every = 2 }");
        assert!(near(run(&mut two, &[clock(4.0)])[0], 2.0));
        let mut hard = evaluator("source = \"clock\"\nhow = \"step\"\nease = \"none\"");
        assert_eq!(run(&mut hard, &[clock(2.9), clock(3.0)]), [2.0, 3.0]);
        let mut wrapped =
            evaluator("source = \"clock\"\nhow = \"step\"\nease = \"none\"\nwhen = { wrap = 4 }");
        assert_eq!(run(&mut wrapped, &[clock(5.5)])[0], 1.0);
    }

    #[test]
    fn pick_keeps_its_index_until_the_label_changes() {
        let mut e = evaluator("source = \"section_label\"\nhow = \"pick\"\nwhen = { options = 3 }");
        let label = |l: f32| MusicValues {
            section_label: l,
            ..a()
        };
        let out = run(
            &mut e,
            &[label(0.0), label(0.0), label(1.0), label(1.0), label(0.0)],
        );
        assert!(
            out.iter()
                .all(|i| (0.0..3.0).contains(i) && i.fract() == 0.0)
        );
        assert_eq!(out[0], out[1]);
        assert_eq!(out[2], out[3]);
        // The same label gets the same one every time it comes, as the transitions' picks do.
        assert_eq!(out[0], out[4]);
        let expected = |l: f32| pick_by_label(&[0u32, 1, 2], l).unwrap() as f32;
        assert_eq!(out[0], expected(0.0));
        assert_eq!(out[2], expected(1.0));
        // From an event source it goes to the next option on each event, round.
        let mut bars = evaluator("source = \"bars\"\nhow = \"pick\"\nwhen = { options = 3 }");
        let bar = MusicValues { bars: 1, ..a() };
        assert_eq!(
            run(&mut bars, &[a(), bar, a(), bar, bar, bar]),
            [0.0, 1.0, 1.0, 2.0, 0.0, 1.0]
        );
    }

    #[test]
    fn accumulate_adds_per_beat_or_per_second_and_wraps() {
        let drive = |m: MusicValues| MusicValues { drive: 1.0, ..m };
        // At 120 bpm a frame of 0.1 s is 0.2 beats.
        let mut e = evaluator("source = \"drive\"\nhow = \"accumulate\"\nwhen = { rate = 2.0 }");
        let out = run(&mut e, &[drive(a()); 3]);
        assert!(near(out[2], 3.0 * 0.2 * 2.0));
        // Per second, the tempo does not matter.
        let mut seconds =
            evaluator("source = \"drive\"\nhow = \"accumulate\"\nwhen = { per = \"second\" }");
        assert!(near(run(&mut seconds, &[drive(a()); 3])[2], 0.3));
        // Without a followed beat it is seconds in either case.
        let mut loose = evaluator("source = \"drive\"\nhow = \"accumulate\"");
        assert!(near(run(&mut loose, &[drive(at(Level::C)); 3])[2], 0.3));
        // It starts at the input's rest and wraps.
        let mut wrapped = Evaluator::new(
            &bind("source = \"drive\"\nhow = \"accumulate\"\nwhen = { per = \"second\", wrap = 0.25 }"),
            &input([0.0, 1.0], Some(0.1)),
        )
        .unwrap();
        let out = run(&mut wrapped, &[drive(a()); 3]);
        assert!(near(out[0], 0.2) && near(out[1], 0.05) && near(out[2], 0.15));
    }

    /// A binding of `bars` (which means something only at A) with a fallback of each kind.
    fn bars_with(fallback: &str) -> Evaluator {
        evaluator(&format!(
            "source = \"bars\"\nhow = \"follow\"\nfallback = {fallback}\n"
        ))
    }

    #[test]
    fn each_fallback_takes_over_where_the_source_means_nothing() {
        let music = |level| MusicValues {
            bars: 1,
            hit: 0.3,
            ..at(level)
        };
        let tick = |e: &mut Evaluator, level| e.tick(0.1, &music(level), level);
        // A source of the same kind, read the same way.
        let mut e = bars_with("\"hit\"");
        let t = tick(&mut e, Level::A);
        assert_eq!((t.used, t.output, t.level), (Used::Source, 1.0, Level::A));
        for level in [Level::B, Level::C] {
            let t = tick(&mut e, level);
            assert_eq!((t.used, t.level), (Used::Fallback("hit"), level));
            assert!(near(t.output, 0.3));
            // The source's own value is still the one reported.
            assert_eq!(t.source, 1.0);
        }
        // A fixed value.
        let mut e = bars_with("{ value = 0.6 }");
        assert_eq!(tick(&mut e, Level::A).output, 1.0);
        for level in [Level::B, Level::C] {
            let t = tick(&mut e, level);
            assert_eq!((t.used, t.output), (Used::Value, 0.6));
        }
        // The input's rest.
        let mut e = Evaluator::new(
            &bind("source = \"bars\"\nhow = \"follow\"\nfallback = \"rest\""),
            &input([0.0, 1.0], Some(0.2)),
        )
        .unwrap();
        assert_eq!(tick(&mut e, Level::A).output, 1.0);
        for level in [Level::B, Level::C] {
            let t = tick(&mut e, level);
            assert_eq!((t.used, t.output), (Used::Rest, 0.2));
        }
        // Hold: the last output stays, and the source is back when the level is.
        let mut e = bars_with("\"hold\"");
        assert_eq!(tick(&mut e, Level::A).output, 1.0);
        for level in [Level::B, Level::C] {
            let t = tick(&mut e, level);
            assert_eq!((t.used, t.output), (Used::Hold, 1.0));
        }
        let mut e = bars_with("\"hold\"");
        // Held before it ever had a value, it holds the rest.
        assert_eq!(tick(&mut e, Level::C).output, 0.0);
        assert_eq!(tick(&mut e, Level::A).used, Used::Source);
        // No fallback at all: the source is used at every level.
        let mut e = evaluator("source = \"bars\"\nhow = \"follow\"");
        for level in Level::ALL {
            let t = tick(&mut e, level);
            assert_eq!((t.used, t.output), (Used::Source, 1.0));
        }
    }

    #[test]
    fn a_source_that_means_something_at_b_falls_back_only_at_c() {
        let mut e = evaluator(
            "source = \"cue_in\"\nhow = \"follow\"\nwhen = { from = [0, 10] }\nfallback = { value = 0.0 }",
        );
        let music = |level| MusicValues {
            cue_in: 5.0,
            ..at(level)
        };
        let used = |e: &mut Evaluator, level| e.tick(0.1, &music(level), level).used;
        assert_eq!(used(&mut e, Level::A), Used::Source);
        assert_eq!(used(&mut e, Level::B), Used::Source);
        assert_eq!(used(&mut e, Level::C), Used::Value);
    }

    #[test]
    fn a_trigger_falls_back_to_another_event_and_stops_at_a_level_without_its_own() {
        let mut e = evaluator(
            "source = \"impacts\"\nhow = \"trigger\"\nfallback = \"hits\"\nwhen = { strength = 0.8 }",
        );
        // `impacts` means something everywhere, so the fallback is never used here.
        let both = MusicValues {
            impacts: 1,
            impact: 0.9,
            hits: 1,
            hit: 0.2,
            ..at(Level::C)
        };
        let t = e.tick(0.1, &both, Level::C);
        assert_eq!((t.used, t.output), (Used::Source, 0.9));
        // `bars` only at A: at C the same bind peaks on the hits instead.
        let mut e = evaluator("source = \"bars\"\nhow = \"trigger\"\nfallback = \"hits\"");
        let hits = MusicValues {
            hits: 1,
            hit: 0.7,
            bars: 0,
            ..at(Level::C)
        };
        let t = e.tick(0.1, &hits, Level::C);
        assert_eq!((t.used, t.output), (Used::Fallback("hits"), 0.7));
        let bar = MusicValues { bars: 1, ..a() };
        assert_eq!(e.tick(0.1, &bar, Level::A).used, Used::Source);
    }

    #[test]
    fn mistakes_name_what_is_wrong() {
        let named = |text: &str, part: &str| {
            let e = error(text);
            assert!(e.contains(part), "{e:?} lacks {part:?}");
            assert!(e.contains("`x`"), "{e:?} does not name the target");
        };
        named(
            "source = \"loudness\"\nhow = \"follow\"",
            "no source `loudness`",
        );
        named("source = \"drive\"\nhow = \"wobble\"", "no how `wobble`");
        named(
            "source = \"drive\"\nhow = \"trigger\"",
            "needs a source that counts events",
        );
        named(
            "source = \"drive\"\nhow = \"follow\"\nwhen = { strength = 0.5 }",
            "unknown key `strength`",
        );
        named(
            "source = \"hits\"\nhow = \"trigger\"\nwhen = { release = 1, rate = 2 }",
            "unknown key `rate`",
        );
        named(
            "source = \"drive\"\nhow = \"follow\"\nfallback = \"nothing\"",
            "no fallback `nothing`",
        );
        named(
            "source = \"hits\"\nhow = \"trigger\"\nfallback = \"drive\"",
            "fallback source that counts events",
        );
        named(
            "source = \"drive\"\nhow = \"follow\"\nwhen = { curve = \"zig\" }",
            "no curve `zig`",
        );
        named(
            "source = \"drive\"\nhow = \"follow\"\nease = \"bounce\"",
            "no ease `bounce`",
        );
        named(
            "source = \"hits\"\nhow = \"trigger\"\nease = \"out\"",
            "`ease` is for follow and step",
        );
        named(
            "source = \"drive\"\nhow = \"follow\"\nwhen = { from = [1, 1] }",
            "different ends",
        );
        named("source = \"drive\"\nhow = \"pick\"", "`when.options`");
        named(
            "source = \"drive\"\nhow = \"pick\"\nwhen = { options = 1.5 }",
            "`when.options`",
        );
        named(
            "source = \"clock\"\nhow = \"step\"\nwhen = { every = 0 }",
            "`when.every`",
        );
        named(
            "source = \"drive\"\nhow = \"follow\"\nwhen = { smooth = \"a\" }",
            "`when.smooth`",
        );
        named(
            "source = \"hits\"\nhow = \"trigger\"\nwhen = { scale = 1 }",
            "`when.scale`",
        );
    }

    #[test]
    fn the_bindings_of_a_scene_drive_their_inputs_and_the_rest_stay_at_rest() {
        let inputs = [
            InputDecl {
                id: "wave".into(),
                ..input([0.0, 2.0], Some(0.5))
            },
            InputDecl {
                id: "idle".into(),
                ..input([0.0, 1.0], Some(0.25))
            },
            InputDecl {
                id: "gate".into(),
                ..input([0.0, 1.0], None)
            },
        ];
        let binds: Vec<Bind> = [
            "target = \"gate\"\nsource = \"hits\"\nhow = \"trigger\"\nwhen = { release = 0 }",
            "target = \"wave\"\nsource = \"drive\"\nhow = \"follow\"",
        ]
        .iter()
        .map(|text| toml::from_str(text).unwrap())
        .collect();
        let mut bindings = Bindings::new(&inputs, &binds).unwrap();
        assert_eq!(bindings.values(), [0.5, 0.25, 0.0]);
        let music = MusicValues {
            drive: 0.5,
            hits: 1,
            hit: 1.0,
            ..a()
        };
        assert_eq!(bindings.tick(0.1, &music), [1.0, 0.25, 1.0]);
        let report = bindings.report();
        assert_eq!(
            report
                .iter()
                .map(|r| (r.target.as_str(), r.source, r.how))
                .collect::<Vec<_>>(),
            [("gate", "hits", "trigger"), ("wave", "drive", "follow")]
        );
        assert_eq!(report[0].tick.level, Level::A);
        // A bind of an input that is not declared is refused.
        let lone: Bind =
            toml::from_str("target = \"nope\"\nsource = \"drive\"\nhow = \"follow\"").unwrap();
        assert!(Bindings::new(&inputs, &[lone]).is_err());
    }
    #[test]
    fn the_report_carries_the_bind_as_written_and_a_renewal_keeps_what_is_the_same() {
        let inputs = [
            InputDecl {
                id: "wave".into(),
                ..input([0.0, 100.0], None)
            },
            InputDecl {
                id: "gate".into(),
                ..input([0.0, 1.0], None)
            },
        ];
        let make = |gate_release: &str| -> Vec<Bind> {
            [
                format!(
                    "target = \"gate\"\nsource = \"impacts\"\nhow = \"trigger\"\nwhen = {{ release = {gate_release} }}\nfallback = \"hits\""
                ),
                "target = \"wave\"\nsource = \"drive\"\nhow = \"accumulate\"\nwhen = { per = \"second\", rate = 4 }\nfallback = { value = 0.5 }".to_string(),
            ]
            .iter()
            .map(|text| toml::from_str(text).unwrap())
            .collect()
        };
        let mut bindings = Bindings::new(&inputs, &make("0.5")).unwrap();
        let report = bindings.report();
        assert_eq!(report[0].when, "{ release = 0.5 }");
        assert_eq!(report[0].fallback, "\"hits\"");
        assert_eq!(report[1].when, "{ per = \"second\", rate = 4 }");
        assert_eq!(report[1].fallback, "{ value = 0.5 }");
        assert_eq!(report[1].ease, "");
        let music = MusicValues { drive: 1.0, ..a() };
        bindings.tick(0.5, &music);
        let walked = bindings.values()[0];
        assert!(walked > 0.0);
        // The gate changed, the wave did not: it keeps its distance, the gate starts over.
        let mut renewed = bindings.renewed(&inputs, &make("1.0")).unwrap();
        assert_eq!(renewed.values()[0], walked);
        assert_eq!(renewed.report()[0].when, "{ release = 1.0 }");
        assert!(renewed.tick(0.5, &music)[0] > walked);
        // A text that does not compile leaves the old as they were.
        let mut wrong = make("0.5");
        wrong[0].source = "loudness".into();
        assert!(bindings.renewed(&inputs, &wrong).is_err());
        assert_eq!(bindings.values()[0], walked);
    }
}
