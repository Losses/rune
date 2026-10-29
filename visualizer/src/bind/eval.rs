//! One evaluator per `[[bind]]`: it owns the binding's state, is ticked every frame with the
//! frame's time step and music, and says the value of the input it drives and which source
//! produced it (see "Bindings" in `docs/music.md`).
//!
//! A binding names a source and a `how`. When the source means nothing at the frame's level (see
//! [`Source::meaningful_at`]) the binding's `fallback` takes over: another source read the same
//! way, a fixed value, the last output, or the input's rest. A binding with no fallback uses its
//! source at every level, as a scene that reads the raw signal does.
//!
//! A source is a name of the registry or `input:<id>`, the value of an input declared before the
//! binding's own. A number of a `when` is a literal or `{ param = "<id>" }`, read every frame.

use anyhow::{Context, Result, bail, ensure};

use super::declared;
use super::level::Level;
use super::sources::{self, Reading, Source};
use crate::course;
use crate::scene::{Bind, FallbackDecl, InputDecl, MusicValues, Param, ParamKind};
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
    /// The engine's exponential ease, or `1 - (1 - x)^p` with an `ease_power`.
    Out,
    /// A smoothstep within each beat.
    Smooth,
}

impl Ease {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "none" => Some(Self::None),
            "out" => Some(Self::Out),
            "smooth" => Some(Self::Smooth),
            _ => None,
        }
    }

    /// The beat time `t` as this ease makes it; `power` is the `ease_power` of `Out`.
    fn apply(self, t: f32, power: Option<f32>) -> f32 {
        match (self, power) {
            (Self::None, _) => t,
            (Self::Out, None) => course::eased(t),
            (Self::Out, Some(p)) => {
                let t = t.max(0.0);
                t.floor() + 1.0 - (1.0 - t.fract()).powf(p.max(1e-3))
            }
            (Self::Smooth, _) => {
                let t = t.max(0.0);
                t.floor() + Curve::Smooth.at(t.fract())
            }
        }
    }
}

/// What a time in a `when` is counted in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Per {
    Beat,
    Second,
}

/// How a trigger's envelope falls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Decay {
    /// A straight fall over `release`.
    Linear,
    /// An exponential fall with `release` as the time constant.
    Exp,
}

/// Where a trigger's envelope starts to fall.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DecayFrom {
    /// In the frame after the event: the event's frame shows the peak.
    Next,
    /// In the event's own frame: the peak is set and the frame's time step already taken off it
    /// (or, with an `attack`, already rising by it), as a scene that wrote `k = 1; k *= exp(-dt / tau)`
    /// itself saw it.
    Event,
}

/// What a counter's `rate` is measured in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Rate {
    /// Per beat of the music (per second where the beat is not followed).
    Beat,
    Second,
    /// Per event of an event source, with no time in it.
    Event,
}

/// A number of a `when`: written in the manifest, or the current value of a parameter as a
/// small expression, `param * mul + add` (a bare `{ param = "id" }` has a `mul` of 1 and an
/// `add` of 0).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Num {
    Const(f32),
    Param { index: usize, mul: f32, add: f32 },
}

/// What a reading is multiplied by before the `how` sees it.
#[derive(Clone, Copy)]
enum Mul {
    /// The value of a source (or of an input).
    Source(Src),
    /// A number of the `when`.
    Num(Num),
}

/// What a frame offers a binding besides the music: the numeric parameters in the manifest's
/// order, the frame's seed, whether the scene cut in it, and the inputs already worked out in
/// this frame.
#[derive(Clone, Copy, Default)]
pub struct Env<'a> {
    pub params: &'a [f32],
    pub seed: u32,
    /// Whether the scene's own director cut to another shot in this frame (what the event
    /// source `cut` counts; the runtime's own bindings have no scene and never see one).
    pub cut: bool,
    inputs: &'a [f32],
}

impl Env<'_> {
    fn num(&self, num: Num) -> f32 {
        match num {
            Num::Const(v) => v,
            Num::Param { index, mul, add } => {
                self.params.get(index).copied().unwrap_or(0.0) * mul + add
            }
        }
    }
}

/// What a binding's names are looked up in: the scene's parameters and inputs.
#[derive(Clone, Copy, Default)]
pub struct Scope<'a> {
    pub params: &'a [Param],
    pub inputs: &'a [InputDecl],
}

/// Where a binding reads from: a source of the registry, or an input declared before it.
#[derive(Clone, Copy)]
enum Src {
    Source(&'static Source),
    Input(usize),
}

impl Src {
    fn read(self, music: &MusicValues, env: &Env) -> Reading {
        match self {
            Self::Source(source) => source.read_in(music, env.cut),
            Self::Input(slot) => Reading {
                value: env.inputs.get(slot).copied().unwrap_or(0.0),
                count: 0,
            },
        }
    }

    fn event(self) -> bool {
        matches!(self, Self::Source(source) if source.event)
    }

    fn meaningful_at(self, level: Level) -> bool {
        match self {
            Self::Source(source) => source.meaningful_at(level),
            Self::Input(_) => true,
        }
    }

    /// The source called `name` (or `input:<id>`, an input before slot `before`).
    fn parse(name: &str, scope: Scope, before: usize) -> Result<Self> {
        if let Some(id) = name.strip_prefix("input:") {
            let slot = scope
                .inputs
                .iter()
                .position(|input| input.id == id)
                .with_context(|| format!("no input `{id}` to read from"))?;
            ensure!(
                slot < before,
                "`input:{id}` must be declared before the input this binds (an input only reads earlier ones)"
            );
            return Ok(Self::Input(slot));
        }
        sources::find(name).map(Self::Source).with_context(|| {
            format!(
                "no source `{name}` (the sources are: {}, or input:<id>)",
                sources::names()
            )
        })
    }
}

/// How an event `pick` chooses its first index.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Start {
    Index(Num),
    /// From the frame's seed.
    Seed,
}

/// How an event `pick` moves to another index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Jump {
    Next,
    Random,
}

/// A binding's `how` with its parameters.
#[derive(Clone, Debug, PartialEq)]
enum Mode {
    Follow {
        from: [Num; 2],
        to: [Num; 2],
        curve: Curve,
        /// A time constant in `per`; 0 for none.
        smooth: Num,
        per: Per,
        ease: Ease,
        power: Option<Num>,
        clamp: bool,
    },
    Trigger {
        threshold: Num,
        /// In `per`.
        attack: Num,
        release: Num,
        per: Per,
        decay: Decay,
        decay_from: DecayFrom,
        scale: bool,
    },
    Step {
        every: Num,
        /// Whether `every` counts bars, not beats.
        bars: bool,
        /// In the source's units, added before counting.
        offset: Num,
        ease: Ease,
        power: Option<Num>,
        wrap: Option<Num>,
    },
    Pick {
        /// The indices 0..options, for [`pick_by_label`].
        options: Vec<u32>,
        /// What an event pick does (a value source goes by its label): the least and the most
        /// strength that counts.
        strength: Num,
        max_strength: Option<Num>,
        every: Num,
        chance: Num,
        start: Start,
        jump: Jump,
    },
    Accumulate {
        rate: Num,
        per: Rate,
        /// For `Rate::Event`: the least strength that counts.
        strength: Num,
        /// An event source that sets the total back to 0.
        reset: Option<&'static Source>,
        wrap: Option<Num>,
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
    /// The value of the binding's own source (whichever means produced the output), after its
    /// `mul` and `add`.
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

/// What a frame's output comes from; the flag says whether the source counts events.
enum Means {
    Source(bool, Reading),
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
    /// An event `pick`: whether the first index is chosen, the section it counts in, and how many
    /// events (past `strength`) it has counted there.
    started: bool,
    section: Option<f32>,
    counted: u32,
    /// The running total (`accumulate`).
    total: f32,
    /// A label `pick` that starts from the seed: the seed of its first frame, which salts the
    /// hash of the labels for as long as the bind lives.
    salt: Option<u32>,
}

/// The evaluator of one `[[bind]]`.
pub struct Evaluator {
    target: String,
    how: &'static str,
    source: Src,
    source_name: &'static str,
    /// A reading is `value * mul + add` before the `how` sees it.
    mul: Option<Mul>,
    add: Num,
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
    scope: Scope<'a>,
}

impl<'a> When<'a> {
    /// A number of the key: a literal, `{ param = "<id>" }`, or `{ param = "<id>", mul = k,
    /// add = c }` (the parameter times `k`, plus `c`); `valid` says whether a literal is
    /// acceptable (a parameter is only known to be numeric here), `rule` what it must be.
    fn num_of(
        &self,
        value: &toml::Value,
        key: &str,
        valid: fn(f32) -> bool,
        rule: &str,
    ) -> Result<Num> {
        let what = format!("`when.{key}`");
        if let toml::Value::Table(table) = value {
            let shape =
                || format!("{what} must be a number or {{ param = \"<id>\", mul = k, add = c }}");
            let id = match table.get("param") {
                Some(toml::Value::String(id)) => id,
                _ => bail!("{}", shape()),
            };
            ensure!(
                table
                    .keys()
                    .all(|key| ["param", "mul", "add"].contains(&key.as_str())),
                "{}",
                shape()
            );
            let part = |key: &str, default: f32| -> Result<f32> {
                table.get(key).map_or(Ok(default), |value| {
                    number(value, &format!("{what}: `{key}` of the parameter"))
                })
            };
            let (mul, add) = (part("mul", 1.0)?, part("add", 0.0)?);
            let index = self
                .scope
                .params
                .iter()
                .position(|param| &param.id == id)
                .with_context(|| format!("{what}: no parameter `{id}`"))?;
            ensure!(
                matches!(
                    self.scope.params[index].kind,
                    ParamKind::Float | ParamKind::Int
                ),
                "{what}: parameter `{id}` is not a number (float or int)"
            );
            return Ok(Num::Param { index, mul, add });
        }
        let v = number(value, &what)?;
        ensure!(valid(v), "{what} {rule}");
        Ok(Num::Const(v))
    }

    fn number(&mut self, key: &'static str, default: f32) -> Result<f32> {
        self.read.push(key);
        match self.table.get(key) {
            Some(value) => number(value, &format!("`when.{key}`")),
            None => Ok(default),
        }
    }

    fn dynamic(
        &mut self,
        key: &'static str,
        default: f32,
        valid: fn(f32) -> bool,
        rule: &str,
    ) -> Result<Num> {
        Ok(self.maybe(key, valid, rule)?.unwrap_or(Num::Const(default)))
    }

    fn maybe(
        &mut self,
        key: &'static str,
        valid: fn(f32) -> bool,
        rule: &str,
    ) -> Result<Option<Num>> {
        self.read.push(key);
        self.table
            .get(key)
            .map(|value| self.num_of(value, key, valid, rule))
            .transpose()
    }

    fn pair(&mut self, key: &'static str) -> Result<Option<[Num; 2]>> {
        self.read.push(key);
        let Some(value) = self.table.get(key) else {
            return Ok(None);
        };
        let what = format!("`when.{key}`");
        let toml::Value::Array(items) = value else {
            bail!("{what} must be [lo, hi]");
        };
        ensure!(items.len() == 2, "{what} must be [lo, hi]");
        let any = |_: f32| true;
        Ok(Some([
            self.num_of(&items[0], key, any, "")?,
            self.num_of(&items[1], key, any, "")?,
        ]))
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

fn non_negative(v: f32) -> bool {
    v >= 0.0
}

fn positive(v: f32) -> bool {
    v > 0.0
}

fn any(_: f32) -> bool {
    true
}

fn unit_range(v: f32) -> bool {
    (0.0..=1.0).contains(&v)
}

impl Evaluator {
    /// The evaluator of `bind`, which drives `input`, with no parameters to read and no other
    /// inputs to read from. Fails, naming what is wrong, if the bind names a source or a `how`
    /// that does not exist, a `when` key its `how` does not take, or a value that makes no sense.
    pub fn new(bind: &Bind, input: &InputDecl) -> Result<Self> {
        Self::new_in(
            bind,
            input,
            Scope {
                params: &[],
                inputs: std::slice::from_ref(input),
            },
        )
    }

    /// As [`Evaluator::new`], with the scene's parameters and inputs to look names up in.
    pub fn new_in(bind: &Bind, input: &InputDecl, scope: Scope) -> Result<Self> {
        Self::compile(bind, input, scope).with_context(|| format!("[[bind]] for `{}`", bind.target))
    }

    fn compile(bind: &Bind, input: &InputDecl, scope: Scope) -> Result<Self> {
        let slot = scope
            .inputs
            .iter()
            .position(|other| other.id == input.id)
            .unwrap_or(scope.inputs.len());
        let source = Src::parse(&bind.source, scope, slot)?;
        let how = HOWS
            .iter()
            .copied()
            .find(|how| *how == bind.how)
            .with_context(|| {
                format!("no how `{}` (it is one of: {})", bind.how, HOWS.join(", "))
            })?;
        if how == "trigger" {
            ensure!(
                source.event(),
                "a trigger needs a source that counts events ({}), not `{}`",
                sources::event_names(),
                bind.source
            );
        }
        let ease = bind
            .ease
            .as_deref()
            .map(|name| {
                Ease::parse(name)
                    .with_context(|| format!("no ease `{name}` (it is one of: none, out, smooth)"))
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
            scope,
        };
        // A source or input by name, or a number (a literal or a parameter).
        when.read.push("mul");
        let mul = match when.table.get("mul") {
            None => None,
            Some(toml::Value::String(name)) => Some(Mul::Source(
                Src::parse(name, scope, slot).context("`when.mul`")?,
            )),
            Some(value) => Some(Mul::Num(when.num_of(value, "mul", any, "")?)),
        };
        let add = when.dynamic("add", 0.0, any, "")?;
        let mode = match how {
            "follow" => {
                let from = when
                    .pair("from")?
                    .unwrap_or([Num::Const(0.0), Num::Const(1.0)]);
                if let [Num::Const(a), Num::Const(b)] = from {
                    ensure!(a != b, "`when.from` must have two different ends");
                }
                let to = when.pair("to")?.unwrap_or([Num::Const(lo), Num::Const(hi)]);
                let curve = match when.word("curve")? {
                    None => Curve::Linear,
                    Some(name) => Curve::parse(name).with_context(|| {
                        format!("no curve `{name}` (it is one of: {})", Curve::NAMES)
                    })?,
                };
                let smooth = when.dynamic("smooth", 0.0, non_negative, "must not be negative")?;
                let per = per_of(&mut when)?;
                let clamp = when.flag("clamp", true)?;
                let power = power_of(&mut when, ease.unwrap_or(Ease::None))?;
                Mode::Follow {
                    from,
                    to,
                    curve,
                    smooth,
                    per,
                    ease: ease.unwrap_or(Ease::None),
                    power,
                    clamp,
                }
            }
            "trigger" => {
                let threshold = when.dynamic("strength", 0.0, any, "")?;
                let attack = when.dynamic("attack", 0.0, non_negative, "must not be negative")?;
                let release =
                    when.dynamic("release", 0.25, non_negative, "must not be negative")?;
                let decay = match when.word("decay")? {
                    None | Some("linear") => Decay::Linear,
                    Some("exp") => Decay::Exp,
                    Some(other) => bail!("no `when.decay` `{other}` (it is linear or exp)"),
                };
                let decay_from = match when.word("decay_from")? {
                    None | Some("next") => DecayFrom::Next,
                    Some("event") => DecayFrom::Event,
                    Some(other) => bail!("no `when.decay_from` `{other}` (it is next or event)"),
                };
                Mode::Trigger {
                    threshold,
                    attack,
                    release,
                    per: per_of(&mut when)?,
                    decay,
                    decay_from,
                    scale: when.flag("scale", true)?,
                }
            }
            "step" => {
                let every = when.dynamic("every", 1.0, positive, "must be above 0")?;
                let bars = match when.word("unit")? {
                    None | Some("beat") => false,
                    Some("bar") => true,
                    Some(other) => bail!("no `when.unit` `{other}` (it is beat or bar)"),
                };
                let offset = when.dynamic("offset", 0.0, any, "")?;
                let ease = ease.unwrap_or(Ease::Out);
                Mode::Step {
                    every,
                    bars,
                    offset,
                    ease,
                    power: power_of(&mut when, ease)?,
                    wrap: wrap_of(&mut when)?,
                }
            }
            "pick" => {
                let options = when.number("options", 0.0)?;
                ensure!(
                    options >= 1.0 && options <= MAX_OPTIONS as f32 && options.fract() == 0.0,
                    "`when.options` must be a whole number from 1 to {MAX_OPTIONS}"
                );
                let strength = when.dynamic("strength", 0.0, any, "")?;
                let max_strength = when.maybe("max_strength", any, "")?;
                let every = when.dynamic("every", 1.0, positive, "must be above 0")?;
                let chance = when.dynamic("chance", 1.0, unit_range, "must be from 0 to 1")?;
                let start = match when.table.get("start") {
                    Some(toml::Value::String(word)) if word == "seed" => {
                        when.read.push("start");
                        Start::Seed
                    }
                    Some(toml::Value::String(other)) => {
                        bail!("no `when.start` `{other}` (it is an index, a parameter or seed)")
                    }
                    _ => Start::Index(when.dynamic(
                        "start",
                        0.0,
                        non_negative,
                        "must not be negative",
                    )?),
                };
                let jump = match when.word("jump")? {
                    None | Some("next") => Jump::Next,
                    Some("random") => Jump::Random,
                    Some(other) => bail!("no `when.jump` `{other}` (it is next or random)"),
                };
                Mode::Pick {
                    options: (0..options as u32).collect(),
                    strength,
                    max_strength,
                    every,
                    chance,
                    start,
                    jump,
                }
            }
            _ => {
                let rate = when.dynamic("rate", 1.0, any, "")?;
                let per = match when.word("per")? {
                    None | Some("beat") => Rate::Beat,
                    Some("second") => Rate::Second,
                    Some("event") => Rate::Event,
                    Some(other) => bail!("no `when.per` `{other}` (it is beat, second or event)"),
                };
                let strength = if per == Rate::Event {
                    when.dynamic("strength", 0.0, any, "")?
                } else {
                    Num::Const(0.0)
                };
                let reset = when
                    .word("reset")?
                    .map(|name| {
                        let found = sources::find(name).with_context(|| {
                            format!(
                                "`when.reset`: no source `{name}` (it is an event source: {})",
                                sources::event_names()
                            )
                        })?;
                        ensure!(
                            found.event,
                            "`when.reset` must be a source that counts events ({}), not `{name}`",
                            sources::event_names()
                        );
                        Ok(found)
                    })
                    .transpose()?;
                Mode::Accumulate {
                    rate,
                    per,
                    strength,
                    reset,
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
        if matches!(
            mode,
            Mode::Accumulate {
                per: Rate::Event,
                ..
            }
        ) {
            ensure!(
                source.event() && !matches!(fallback, Fallback::Source(s) if !s.event),
                "`when.per = \"event\"` counts the events of the source, so the source (and a fallback source) must count events ({}), not `{}`",
                sources::event_names(),
                bind.source
            );
        }
        if how == "pick" {
            let event = source.event() || matches!(fallback, Fallback::Source(s) if s.event);
            let set = |key: &str| bind.when.contains_key(key);
            ensure!(
                event
                    || !["strength", "max_strength", "every", "chance", "jump"]
                        .iter()
                        .any(|k| set(k)),
                "`strength`, `max_strength`, `every`, `chance` and `jump` are for a pick of an event source"
            );
            ensure!(
                event
                    || bind
                        .when
                        .get("start")
                        .is_none_or(|v| v.as_str() == Some("seed")),
                "`start` is for a pick of an event source, except `start = \"seed\"`, which a pick by label takes too"
            );
        }
        let rest = input.rest();
        Ok(Self {
            target: bind.target.clone(),
            how,
            source,
            source_name: intern(&bind.source),
            mul,
            add,
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
        self.source_name
    }

    pub fn how(&self) -> &'static str {
        self.how
    }

    /// What it said in its last frame.
    pub fn last(&self) -> Tick {
        self.tick
    }

    /// Advance by a frame of `dt` seconds in `music`, which is at `level`, and say the output;
    /// with no parameters, no seed and no other inputs to read (see [`Evaluator::tick_in`]).
    pub fn tick(&mut self, dt: f32, music: &MusicValues, level: Level) -> Tick {
        self.tick_in(dt, music, level, &Env::default())
    }

    /// As [`Evaluator::tick`], in a frame whose parameters, seed and inputs are `env`.
    pub fn tick_in(&mut self, dt: f32, music: &MusicValues, level: Level, env: &Env) -> Tick {
        let own = self.modulated(self.source.read(music, env), music, env);
        let means = match self.fallback {
            Fallback::None => Means::Source(self.source.event(), own),
            _ if self.source.meaningful_at(level) => Means::Source(self.source.event(), own),
            Fallback::Source(other) => Means::Fallback(
                other,
                self.modulated(other.read_in(music, env.cut), music, env),
            ),
            Fallback::Value(value) => Means::Value(value),
            Fallback::Hold => Means::Hold,
            Fallback::Rest => Means::Rest,
        };
        let (used, output) = match means {
            Means::Source(event, reading) => {
                (Used::Source, self.advance(dt, music, env, event, reading))
            }
            Means::Fallback(source, reading) => (
                Used::Fallback(source.name),
                self.advance(dt, music, env, source.event, reading),
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

    /// A reading as `mul` and `add` make it.
    fn modulated(&self, mut reading: Reading, music: &MusicValues, env: &Env) -> Reading {
        match self.mul {
            None => {}
            Some(Mul::Source(source)) => reading.value *= source.read(music, env).value,
            Some(Mul::Num(num)) => reading.value *= env.num(num),
        }
        reading.value += env.num(self.add);
        reading
    }

    /// Move the state on a frame with what a source said (`event`: it counts events), and give
    /// the output.
    fn advance(
        &mut self,
        dt: f32,
        music: &MusicValues,
        env: &Env,
        event: bool,
        reading: Reading,
    ) -> f32 {
        let beats = beats_in(dt, music);
        match &self.mode {
            Mode::Follow {
                from,
                to,
                curve,
                smooth,
                per,
                ease,
                power,
                clamp,
            } => {
                let x = ease.apply(reading.value, power.map(|p| env.num(p)));
                let (from, to) = (from.map(|n| env.num(n)), to.map(|n| env.num(n)));
                let span = from[1] - from[0];
                let t = if span == 0.0 {
                    0.0
                } else {
                    (x - from[0]) / span
                };
                // Beyond the ends the map goes straight on unless it is clamped.
                let shaped = if *clamp {
                    curve.at(t)
                } else {
                    curve.at(t) + (t - t.clamp(0.0, 1.0))
                };
                let target = to[0] + shaped * (to[1] - to[0]);
                let smooth = env.num(*smooth).max(0.0);
                let by = match per {
                    Per::Beat => beats,
                    Per::Second => dt,
                };
                let level = match (self.state.level, smooth > 0.0) {
                    (Some(level), true) => level + (target - level) * (1.0 - (-by / smooth).exp()),
                    _ => target,
                };
                self.state.level = Some(level);
                level
            }
            Mode::Trigger {
                threshold,
                attack,
                release,
                per,
                decay,
                decay_from,
                scale,
            } => {
                let by = match per {
                    Per::Beat => beats,
                    Per::Second => dt,
                };
                let (attack, release) = (env.num(*attack).max(0.0), env.num(*release).max(0.0));
                let mut level = self.state.level.unwrap_or(0.0);
                let fired = reading.count > 0 && reading.value >= env.num(*threshold);
                if fired {
                    self.state.peak = if *scale {
                        reading.value.clamp(0.0, 1.0)
                    } else {
                        1.0
                    };
                    self.state.rising = attack > 0.0;
                    if !self.state.rising {
                        level = self.state.peak;
                    }
                }
                // On the event's frame the envelope moves on only when it is asked to: with
                // `decay_from = "event"` it falls (or, with an attack, starts to rise) by this
                // frame's step at once. With a `release` of 0 there is nothing to fall by, so
                // the peak shows for the one frame either way.
                if !fired || *decay_from == DecayFrom::Event {
                    if self.state.rising {
                        level = (level + self.state.peak * by / attack).min(self.state.peak);
                        self.state.rising = level < self.state.peak;
                    } else if release > 0.0 {
                        level = match decay {
                            Decay::Linear => (level - self.state.peak * by / release).max(0.0),
                            Decay::Exp => {
                                let level = level * (-by / release).exp();
                                if level < 1e-3 { 0.0 } else { level }
                            }
                        };
                    } else if !fired {
                        level = 0.0;
                    }
                }
                self.state.level = Some(level);
                self.rest + level * (self.top - self.rest)
            }
            Mode::Step {
                every,
                bars,
                offset,
                ease,
                power,
                wrap,
            } => {
                let unit = if *bars { music.meter.max(1.0) } else { 1.0 };
                let every = env.num(*every).max(1e-6) * unit;
                let t = (reading.value + env.num(*offset)).max(0.0) / every;
                let counted = match ease {
                    Ease::None => t.floor(),
                    _ => ease.apply(t, power.map(|p| env.num(p))),
                };
                wrap.map_or(counted, |wrap| counted.rem_euclid(env.num(wrap).max(1e-6)))
            }
            Mode::Pick {
                options,
                strength,
                max_strength,
                every,
                chance,
                start,
                jump,
            } => {
                let len = options.len().max(1) as u32;
                if event {
                    if !self.state.started {
                        self.state.started = true;
                        self.state.index = match start {
                            Start::Index(n) => env.num(*n).round().max(0.0) as u32 % len,
                            Start::Seed => hash(env.seed, 0, 2) % len,
                        };
                    }
                    if self.state.section != Some(music.section) {
                        self.state.section = Some(music.section);
                        self.state.counted = 0;
                    }
                    let every = env.num(*every).round().max(1.0) as u32;
                    let chance = env.num(*chance);
                    if reading.value >= env.num(*strength)
                        && max_strength.is_none_or(|max| reading.value <= env.num(max))
                    {
                        for _ in 0..reading.count {
                            self.state.counted += 1;
                            let n = self.state.counted;
                            let section = music.section as u32;
                            if !n.is_multiple_of(every)
                                || (chance < 1.0 && unit_hash(section, n, 3) >= chance)
                            {
                                continue;
                            }
                            self.state.index = match jump {
                                Jump::Next => (self.state.index + 1) % len,
                                // Any other index, by the section and the event's number in it.
                                Jump::Random if len > 1 => {
                                    (self.state.index + 1 + hash(section, n, 1) % (len - 1)) % len
                                }
                                Jump::Random => 0,
                            };
                        }
                    }
                } else if self.state.seen != Some(reading.value) {
                    self.state.seen = Some(reading.value);
                    self.state.index = match start {
                        // The seed of the bind's first frame salts the hash, so the same label
                        // gets the same option all the play through, and another play may not.
                        Start::Seed => {
                            let salt = *self.state.salt.get_or_insert(env.seed);
                            hash(reading.value.max(0.0) as u32, salt, 4) % len
                        }
                        Start::Index(_) => {
                            pick_by_label(options, reading.value).unwrap_or_default()
                        }
                    };
                }
                self.state.index as f32
            }
            Mode::Accumulate {
                rate,
                per,
                strength,
                reset,
                wrap,
            } => {
                let rate = env.num(*rate);
                let step = match per {
                    Rate::Beat => reading.value * rate * (dt * beats_per_second(music)),
                    Rate::Second => reading.value * rate * dt,
                    // Each event of a frame that is strong enough adds the rate, with no time.
                    Rate::Event if reading.value >= env.num(*strength) => {
                        reading.count as f32 * rate
                    }
                    Rate::Event => 0.0,
                };
                let total = self.state.total + step;
                self.state.total =
                    wrap.map_or(total, |wrap| total.rem_euclid(env.num(wrap).max(1e-6)));
                // A reset comes after this frame is added: on its event the sum is 0, so what
                // starts over (a camera's drift at a cut) starts where it should, as a scene that
                // summed and then zeroed on the event saw it.
                if reset.is_some_and(|source| source.read_in(music, env.cut).count > 0) {
                    self.state.total = 0.0;
                }
                self.state.total
            }
        }
    }
}

/// A name as a `&'static str`: a source's own, or an `input:<id>` kept once however often the
/// scene is loaded, so that a report can name it as it names the registry's sources.
fn intern(name: &str) -> &'static str {
    use std::collections::HashSet;
    use std::sync::{LazyLock, Mutex};
    static NAMES: LazyLock<Mutex<HashSet<&'static str>>> = LazyLock::new(Mutex::default);
    let mut names = NAMES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(known) = names.get(name) {
        return known;
    }
    let name: &'static str = Box::leak(name.to_owned().into_boxed_str());
    names.insert(name);
    name
}

/// A deterministic mix of three numbers (a section, an event's number in it, a purpose).
fn hash(a: u32, b: u32, salt: u32) -> u32 {
    let mut x =
        a.wrapping_mul(0x9e37_79b1) ^ b.wrapping_mul(0x85eb_ca6b) ^ salt.wrapping_mul(0xc2b2_ae35);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

/// [`hash`] as a number from 0 to just under 1.
fn unit_hash(a: u32, b: u32, salt: u32) -> f32 {
    (hash(a, b, salt) >> 8) as f32 / (1u32 << 24) as f32
}

fn wrap_of(when: &mut When) -> Result<Option<Num>> {
    when.maybe("wrap", positive, "must be above 0")
}

fn per_of(when: &mut When) -> Result<Per> {
    Ok(match when.word("per")? {
        None | Some("beat") => Per::Beat,
        Some("second") => Per::Second,
        Some(other) => bail!("no `when.per` `{other}` (it is beat or second)"),
    })
}

/// The `ease_power`, which only an `out` ease takes.
fn power_of(when: &mut When, ease: Ease) -> Result<Option<Num>> {
    let power = when.maybe("ease_power", positive, "must be above 0")?;
    ensure!(
        power.is_none() || ease == Ease::Out,
        "`when.ease_power` is for `ease = \"out\"`"
    );
    Ok(power)
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
    /// The order they run in: by the place of their input, so that an input is worked out before
    /// the ones that read it.
    order: Vec<usize>,
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
    /// The evaluators of `binds`, which drive `inputs`, with no parameters to read. Fails on a
    /// bind of no input or one that does not compile (see [`Evaluator::new`]).
    pub fn new(inputs: &[InputDecl], binds: &[Bind]) -> Result<Self> {
        Self::with_scope(inputs, &[], binds)
    }

    /// As [`Bindings::new`], where a bind may read the scene's numeric `params`.
    pub fn with_scope(inputs: &[InputDecl], params: &[Param], binds: &[Bind]) -> Result<Self> {
        let scope = Scope { params, inputs };
        let evaluators: Vec<(usize, Evaluator)> = binds
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
                Ok((slot, Evaluator::new_in(bind, &inputs[slot], scope)?))
            })
            .collect::<Result<_>>()?;
        let mut order: Vec<usize> = (0..evaluators.len()).collect();
        order.sort_by_key(|i| evaluators[*i].0);
        Ok(Self {
            evaluators,
            order,
            values: inputs.iter().map(InputDecl::rest).collect(),
        })
    }

    /// The bindings of `binds` for `inputs`, where a binding that is the same as one of these
    /// (the same bind of the same input) takes over its evaluator and so its state (an envelope,
    /// an accumulated distance) instead of starting over. Fails as [`Bindings::new`] does, and
    /// then these are left as they were.
    pub fn renewed(&mut self, inputs: &[InputDecl], binds: &[Bind]) -> Result<Self> {
        self.renewed_in(inputs, &[], binds)
    }

    /// As [`Bindings::renewed`], where a bind may read the scene's numeric `params` (see
    /// [`Bindings::with_scope`]).
    pub fn renewed_in(
        &mut self,
        inputs: &[InputDecl],
        params: &[Param],
        binds: &[Bind],
    ) -> Result<Self> {
        let mut next = Self::with_scope(inputs, params, binds)?;
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
        self.tick_in(dt, music, &[], 0)
    }

    /// As [`Bindings::tick`], in a frame with the current values of the scene's parameters and
    /// the frame's `seed`.
    pub fn tick_in(&mut self, dt: f32, music: &MusicValues, params: &[f32], seed: u32) -> &[f32] {
        self.tick_cut(dt, music, params, seed, false)
    }

    /// As [`Bindings::tick_in`], in a frame where the scene's director did (`cut`) or did not
    /// cut to another shot, which the event source `cut` counts.
    pub fn tick_cut(
        &mut self,
        dt: f32,
        music: &MusicValues,
        params: &[f32],
        seed: u32,
        cut: bool,
    ) -> &[f32] {
        let level = Level::of(music);
        for &i in &self.order {
            let (slot, evaluator) = &mut self.evaluators[i];
            let env = Env {
                params,
                seed,
                cut,
                inputs: &self.values,
            };
            let output = evaluator.tick_in(dt, music, level, &env).output;
            self.values[*slot] = output;
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
                source: evaluator.source_name,
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

    fn param(text: &str) -> Param {
        toml::from_str(text).unwrap()
    }

    fn params() -> Vec<Param> {
        vec![
            param("id = \"gain\"\ndefault = 1.0"),
            param("id = \"count\"\ntype = \"int\"\ndefault = 3"),
            param("id = \"on\"\ntype = \"bool\"\ndefault = true"),
        ]
    }

    fn with_params(text: &str) -> Result<Evaluator> {
        let params = params();
        Evaluator::new_in(
            &bind(text),
            &input([0.0, 1.0], None),
            Scope {
                params: &params,
                inputs: &[],
            },
        )
    }

    fn env<'a>(params: &'a [f32], seed: u32) -> Env<'a> {
        Env {
            params,
            seed,
            cut: false,
            inputs: &[],
        }
    }

    #[test]
    fn a_param_reference_is_read_every_frame_and_checked_at_load() {
        let mut e = with_params(
            "source = \"drive\"\nhow = \"follow\"\nwhen = { to = [0, { param = \"gain\" }] }",
        )
        .unwrap();
        let music = MusicValues { drive: 0.5, ..a() };
        let mut out = |gain: f32| {
            e.tick_in(0.1, &music, Level::A, &env(&[gain, 0.0, 0.0], 0))
                .output
        };
        assert!(near(out(1.0), 0.5));
        // The scene's value now is the one it reads, as the app changes it.
        assert!(near(out(4.0), 2.0));
        assert!(near(out(0.5), 0.25));
        // An int parameter is a number too, and any number of a `when` may be one.
        let mut e = with_params(
            "source = \"hits\"\nhow = \"trigger\"\nwhen = { strength = { param = \"gain\" } }",
        )
        .unwrap();
        let hit = hit(0.6);
        assert_eq!(
            e.tick_in(0.1, &hit, Level::A, &env(&[0.9, 0.0, 0.0], 0))
                .output,
            0.0
        );
        assert!(near(
            e.tick_in(0.1, &hit, Level::A, &env(&[0.5, 0.0, 0.0], 0))
                .output,
            0.6
        ));
        let err = |text: &str| format!("{:#}", with_params(text).err().expect(text));
        assert!(
            err("source = \"drive\"\nhow = \"follow\"\nwhen = { smooth = { param = \"nope\" } }")
                .contains("no parameter `nope`")
        );
        assert!(
            err("source = \"drive\"\nhow = \"follow\"\nwhen = { smooth = { param = \"on\" } }")
                .contains("not a number")
        );
        assert!(
            err("source = \"drive\"\nhow = \"follow\"\nwhen = { smooth = { thing = 1 } }")
                .contains("{ param")
        );
        // Without the scene's parameters to look in there is none to find.
        assert!(
            Evaluator::new(
                &bind(
                    "source = \"drive\"\nhow = \"follow\"\nwhen = { smooth = { param = \"gain\" } }"
                ),
                &input([0.0, 1.0], None)
            )
            .is_err()
        );
    }

    #[test]
    fn an_exponential_release_has_the_release_as_its_time_constant() {
        let quiet = MusicValues { hit: 0.0, ..a() };
        let text = |extra: &str| {
            format!("source = \"hits\"\nhow = \"trigger\"\nwhen = {{ release = 1.0{extra} }}")
        };
        let mut e = evaluator(&text(", decay = \"exp\""));
        run(&mut e, &[hit(1.0)]);
        // One beat (0.5 s at 120 bpm) is one time constant.
        let out = run(&mut e, &[quiet; 5]);
        assert!(near(out[4], (-1.0f32).exp()), "{out:?}");
        // A fall that never quite reaches nothing is cut off at a thousandth.
        assert_eq!(run(&mut e, &[quiet; 60])[59], 0.0);
        // Linear is the default and ends in a beat.
        let mut linear = evaluator(&text(""));
        run(&mut linear, &[hit(1.0)]);
        assert_eq!(run(&mut linear, &[quiet; 6])[5], 0.0);
        // In seconds, the tempo does not count: 0.1 s of a 1 s release.
        let mut seconds = evaluator(&text(", per = \"second\""));
        run(&mut seconds, &[hit(1.0)]);
        assert!(near(run(&mut seconds, &[quiet])[0], 0.9));
        // The attack is in the same unit.
        let mut slow = evaluator(
            "source = \"hits\"\nhow = \"trigger\"\nwhen = { attack = 1.0, per = \"second\" }",
        );
        run(&mut slow, &[hit(1.0)]);
        assert!(near(run(&mut slow, &[quiet])[0], 0.1));
        assert!(
            error("source = \"hits\"\nhow = \"trigger\"\nwhen = { decay = \"fast\" }")
                .contains("no `when.decay`")
        );
    }

    fn two_inputs() -> [InputDecl; 2] {
        [
            InputDecl {
                id: "first".into(),
                ..input([0.0, 1.0], None)
            },
            InputDecl {
                id: "second".into(),
                ..input([0.0, 10.0], None)
            },
        ]
    }

    fn binds(texts: &[&str]) -> Vec<Bind> {
        texts.iter().map(|t| toml::from_str(t).unwrap()).collect()
    }

    #[test]
    fn an_input_is_a_source_for_the_inputs_after_it_and_works_out_first() {
        let inputs = two_inputs();
        // The reading bind is listed first, and still sees the value of this very frame.
        let binds = binds(&[
            "target = \"second\"\nsource = \"input:first\"\nhow = \"follow\"\nwhen = { from = [0, 1], to = [0, 10] }",
            "target = \"first\"\nsource = \"drive\"\nhow = \"follow\"",
        ]);
        let mut bindings = Bindings::new(&inputs, &binds).unwrap();
        let music = MusicValues { drive: 0.5, ..a() };
        assert_eq!(bindings.tick(0.1, &music), [0.5, 5.0]);
        let report = bindings.report();
        assert_eq!(
            (report[0].target.as_str(), report[0].source),
            ("second", "input:first")
        );
        // An input is not an event, so a trigger cannot take one.
        let trigger = binds_of("second", "input:first", "trigger");
        let e = format!("{:#}", Bindings::new(&inputs, &trigger).err().unwrap());
        assert!(e.contains("needs a source that counts events"), "{e}");
        // Only an earlier input: itself, a later one and one that is not there are refused.
        for (target, source, part) in [
            ("first", "input:second", "declared before"),
            ("first", "input:first", "declared before"),
            ("second", "input:nope", "no input `nope`"),
        ] {
            let b = binds_of(target, source, "follow");
            let e = format!("{:#}", Bindings::new(&inputs, &b).err().unwrap());
            assert!(e.contains(part), "{e}");
        }
    }

    fn binds_of(target: &str, source: &str, how: &str) -> Vec<Bind> {
        binds(&[&format!(
            "target = \"{target}\"\nsource = \"{source}\"\nhow = \"{how}\""
        )])
    }

    #[test]
    fn a_reading_is_multiplied_and_added_to_before_the_how() {
        let music = MusicValues {
            drive: 0.5,
            intensity: 0.4,
            ..a()
        };
        let mut e = evaluator(
            "source = \"drive\"\nhow = \"follow\"\nwhen = { mul = \"intensity\", add = 0.1 }",
        );
        assert!(near(run(&mut e, &[music])[0], 0.5 * 0.4 + 0.1));
        // Alone, `add` shifts, and it may be a parameter.
        let mut shifted = with_params(
            "source = \"drive\"\nhow = \"follow\"\nwhen = { add = { param = \"gain\" } }",
        )
        .unwrap();
        let t = shifted.tick_in(0.1, &music, Level::A, &env(&[0.25, 0.0, 0.0], 0));
        assert!(near(t.output, 0.75) && near(t.source, 0.75));
        // Tempo scaled by drive makes an accumulate: 120 bpm is 2 beats a second, so a frame
        // of 0.1 s with drive 0.5 moves it by 0.1.
        let mut tempo = evaluator(
            "source = \"tempo\"\nhow = \"accumulate\"\nwhen = { mul = \"drive\", per = \"second\", rate = 0.01 }",
        );
        assert!(near(run(&mut tempo, &[music])[0], 120.0 * 0.5 * 0.01 * 0.1));
        // The factor may be an input: a range driven by another input.
        let inputs = two_inputs();
        let binds = binds(&[
            "target = \"first\"\nsource = \"drive\"\nhow = \"follow\"",
            "target = \"second\"\nsource = \"intensity\"\nhow = \"follow\"\nwhen = { mul = \"input:first\" }",
        ]);
        let mut bindings = Bindings::new(&inputs, &binds).unwrap();
        // intensity 0.4 times a first of 0.5, in a 0..10 range.
        assert!(near(bindings.tick(0.1, &music)[1], 0.2 * 10.0));
        let e = error("source = \"drive\"\nhow = \"follow\"\nwhen = { mul = \"loudness\" }");
        assert!(
            e.contains("`when.mul`") && e.contains("no source `loudness`"),
            "{e}"
        );
    }

    #[test]
    fn the_ease_takes_a_power_or_a_smoothstep() {
        let clock = |t: f32| MusicValues { clock: t, ..a() };
        let step = |extra: &str| evaluator(&format!("source = \"clock\"\nhow = \"step\"\n{extra}"));
        // The engine's own ease is the default; a power p is 1 - (1 - x)^p inside each beat.
        let mut quartic = step("when = { ease_power = 4 }");
        let out = run(&mut quartic, &[clock(2.5), clock(3.0)]);
        assert!(near(out[0], 2.0 + 1.0 - 0.5f32.powi(4)) && near(out[1], 3.0));
        let mut plain = step("");
        assert!(near(run(&mut plain, &[clock(0.5)])[0], course::eased(0.5)));
        let mut smooth = step("ease = \"smooth\"");
        let out = run(&mut smooth, &[clock(1.25), clock(1.5), clock(2.0)]);
        assert!(near(out[0], 1.0 + 0.15625) && near(out[1], 1.5) && near(out[2], 2.0));
        // On a follow it is the same, and a power needs the out ease.
        let mut follow = evaluator(
            "source = \"clock\"\nhow = \"follow\"\nease = \"out\"\nwhen = { from = [0, 1], ease_power = 2 }",
        );
        assert!(near(run(&mut follow, &[clock(0.5)])[0], 0.75));
        for text in [
            "source = \"clock\"\nhow = \"follow\"\nwhen = { ease_power = 2 }",
            "source = \"clock\"\nhow = \"step\"\nease = \"smooth\"\nwhen = { ease_power = 2 }",
            "source = \"clock\"\nhow = \"step\"\nwhen = { ease_power = 0 }",
        ] {
            assert!(error(text).contains("ease_power"), "{text}");
        }
    }

    /// Events in `frames` of music, one frame each: the index after each.
    fn picks(text: &str, frames: &[MusicValues]) -> Vec<u32> {
        let mut e = evaluator(&format!(
            "source = \"bars\"\nhow = \"pick\"\nwhen = {{ options = 5, {text} }}"
        ));
        run(&mut e, frames).iter().map(|v| *v as u32).collect()
    }

    fn bars(n: usize, strength: f32, section: f32) -> Vec<MusicValues> {
        vec![
            MusicValues {
                bars: 1,
                section,
                ..a()
            };
            n
        ]
        .into_iter()
        .map(|m| MusicValues { hit: strength, ..m })
        .collect()
    }

    #[test]
    fn a_pick_counts_only_the_events_that_pass_its_strength_every_and_chance() {
        // Strength: a quiet hit is not counted (the strength is the event's `hit`).
        let mut e =
            evaluator("source = \"hits\"\nhow = \"pick\"\nwhen = { options = 3, strength = 0.5 }");
        assert_eq!(
            run(&mut e, &[hit(0.2), hit(0.8), hit(0.4), hit(0.5)]),
            [0.0, 1.0, 1.0, 2.0]
        );
        // Every: only the 2nd, 4th ... counted event moves it.
        assert_eq!(picks("every = 2", &bars(5, 1.0, 0.0)), [0, 1, 1, 2, 2]);
        // Chance 0 never moves it; 1 is always.
        assert_eq!(picks("chance = 0", &bars(6, 1.0, 0.0)), [0; 6]);
        assert_eq!(picks("chance = 1", &bars(3, 1.0, 0.0)), [1, 2, 3]);
        // In between it is the hash of the section and the event's number: the same song
        // repeats itself, and a section of its own has its own pattern.
        let one = picks("chance = 0.5", &bars(40, 1.0, 0.0));
        assert_eq!(one, picks("chance = 0.5", &bars(40, 1.0, 0.0)));
        let moves = one.windows(2).filter(|w| w[0] != w[1]).count();
        assert!((8..=32).contains(&moves), "{moves}");
        assert_ne!(one, picks("chance = 0.5", &bars(40, 1.0, 3.0)));
        // The count starts over in each section, so coming back to one gives the same moves.
        let mut both = bars(10, 1.0, 0.0);
        both.extend(bars(10, 1.0, 1.0));
        both.extend(bars(10, 1.0, 0.0));
        let out = picks("chance = 0.5, jump = \"next\"", &both);
        let steps = |range: std::ops::Range<usize>, from: u32| -> Vec<u32> {
            range
                .map(|i| (out[i] + 5 - if i == 0 { from } else { out[i - 1] }) % 5)
                .collect()
        };
        assert_eq!(steps(0..10, 0), steps(20..30, out[19]));
    }

    #[test]
    fn a_pick_starts_where_it_is_told_and_jumps_next_or_random() {
        let quiet = [a(); 1];
        assert_eq!(picks("start = 3", &quiet), [3]);
        // The next moves on from the start, round.
        assert_eq!(picks("start = 4", &bars(2, 1.0, 0.0)), [0, 1]);
        // From the seed: the same seed gives the same first look, and seeds spread over the
        // options.
        let seeded = |seed: u32| {
            let mut e = evaluator(
                "source = \"bars\"\nhow = \"pick\"\nwhen = { options = 5, start = \"seed\" }",
            );
            e.tick_in(0.1, &a(), Level::A, &env(&[], seed)).output
        };
        assert_eq!(seeded(7), seeded(7));
        let seen: std::collections::HashSet<u32> = (0..64).map(|s| seeded(s) as u32).collect();
        assert!(seen.len() >= 4 && seen.iter().all(|i| *i < 5), "{seen:?}");
        // Random is another option every time, the same ones on every run of the song.
        let out = picks("jump = \"random\"", &bars(30, 1.0, 2.0));
        assert!(out.windows(2).all(|w| w[0] != w[1] && w[1] < 5));
        assert!(out[0] != 0);
        assert_eq!(out, picks("jump = \"random\"", &bars(30, 1.0, 2.0)));
        assert!(out.iter().collect::<std::collections::HashSet<_>>().len() >= 4);
        // These keys are for events: a pick by label refuses them, a bad word is named.
        assert!(
            error("source = \"section_label\"\nhow = \"pick\"\nwhen = { options = 3, every = 2 }")
                .contains("for a pick of an event source")
        );
        assert!(
            error("source = \"bars\"\nhow = \"pick\"\nwhen = { options = 3, jump = \"far\" }")
                .contains("no `when.jump`")
        );
        assert!(
            error("source = \"bars\"\nhow = \"pick\"\nwhen = { options = 3, start = \"later\" }")
                .contains("no `when.start`")
        );
        assert!(
            error("source = \"bars\"\nhow = \"pick\"\nwhen = { options = 3, chance = 2 }")
                .contains("from 0 to 1")
        );
    }

    #[test]
    fn a_step_can_be_offset_and_count_in_bars() {
        let at4 = |t: f32| MusicValues {
            clock: t,
            meter: 4.0,
            ..a()
        };
        let mut offset = evaluator(
            "source = \"clock\"\nhow = \"step\"\nease = \"none\"\nwhen = { offset = 1.5 }",
        );
        // The source's units: 2.4 + 1.5 beats is the 3rd.
        assert_eq!(run(&mut offset, &[at4(0.0), at4(2.4)]), [1.0, 3.0]);
        // Every bar, in beats of a bar of 4, with the offset in beats.
        let mut bar = evaluator(
            "source = \"clock\"\nhow = \"step\"\nease = \"none\"\nwhen = { every = 1, unit = \"bar\" }",
        );
        assert_eq!(
            run(&mut bar, &[at4(3.9), at4(4.0), at4(8.5)]),
            [0.0, 1.0, 2.0]
        );
        let mut two = evaluator(
            "source = \"clock\"\nhow = \"step\"\nease = \"none\"\nwhen = { every = 2, unit = \"bar\", offset = 4 }",
        );
        assert_eq!(run(&mut two, &[at4(3.9), at4(4.0)]), [0.0, 1.0]);
        assert!(
            error("source = \"clock\"\nhow = \"step\"\nwhen = { unit = \"year\" }")
                .contains("no `when.unit`")
        );
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

    #[test]
    fn a_trigger_can_start_to_fall_on_the_frame_of_the_event() {
        let quiet = MusicValues { hit: 0.0, ..a() };
        let trigger = |when: &str| {
            evaluator(&format!(
                "source = \"hits\"\nhow = \"trigger\"\nwhen = {{ {when} }}"
            ))
        };
        // By default the event's frame shows the peak and the fall starts after it; at 120 bpm a
        // frame of 0.1 s is 0.2 beats.
        let mut next = trigger("release = 1.0");
        let out = run(&mut next, &[hit(0.8), quiet]);
        assert!(near(out[0], 0.8) && near(out[1], 0.8 * 0.8));
        assert_eq!(
            trigger("release = 1.0, decay_from = \"next\"").how(),
            "trigger"
        );
        // From the event the same frame has already fallen by its step, as `k = 1; k *= ...`.
        let mut event = trigger("release = 1.0, decay_from = \"event\"");
        let out = run(&mut event, &[hit(0.8), quiet]);
        assert!(near(out[0], 0.8 * 0.8) && near(out[1], 0.8 * 0.6));
        // An exponential fall: the time constant, here in seconds, is taken off the peak at once.
        let mut exp =
            trigger("release = 0.5, decay = \"exp\", per = \"second\", decay_from = \"event\"");
        let out = run(&mut exp, &[hit(1.0), quiet]);
        assert!(near(out[0], (-0.2f32).exp()) && near(out[1], (-0.4f32).exp()));
        // With an attack the rise starts on the event's frame instead of the one after.
        let mut rising = trigger("attack = 1.0, release = 1.0, decay_from = \"event\"");
        let out = run(&mut rising, &[hit(0.8), quiet, quiet]);
        assert!(near(out[0], 0.16) && near(out[1], 0.32) && near(out[2], 0.48));
        let mut late = trigger("attack = 1.0, release = 1.0");
        let out = run(&mut late, &[hit(0.8), quiet, quiet]);
        assert!(near(out[0], 0.0) && near(out[1], 0.16) && near(out[2], 0.32));
        // A release of 0 has nothing to fall by: the peak is one frame long either way.
        let mut blink = trigger("release = 0.0, decay_from = \"event\"");
        let out = run(&mut blink, &[hit(0.8), quiet]);
        assert!(near(out[0], 0.8) && out[1] == 0.0);
        assert!(
            error("source = \"hits\"\nhow = \"trigger\"\nwhen = { decay_from = \"later\" }")
                .contains("no `when.decay_from`")
        );
    }

    #[test]
    fn an_accumulate_can_count_events_without_wrapping_or_a_clock() {
        let events = |hits: u32, strength: f32| MusicValues {
            hits,
            hit: strength,
            ..a()
        };
        // Each event adds the rate, whatever the frame's length; a quiet frame adds nothing.
        let mut e = evaluator(
            "source = \"hits\"\nhow = \"accumulate\"\nwhen = { per = \"event\", rate = 2.0, strength = 0.5 }",
        );
        let out = run(
            &mut e,
            &[
                events(3, 0.8),
                events(2, 0.3),
                events(0, 0.9),
                events(1, 0.5),
            ],
        );
        assert_eq!(out, [6.0, 6.0, 6.0, 8.0]);
        // The default rate is 1, and nothing wraps: the count goes past a pick's 64 options.
        let mut long =
            evaluator("source = \"hits\"\nhow = \"accumulate\"\nwhen = { per = \"event\" }");
        let out = run(&mut long, &vec![events(1, 1.0); 100]);
        assert_eq!(out[99], 100.0);
        // A wrap is still there when it is wanted.
        let mut wrapped = evaluator(
            "source = \"hits\"\nhow = \"accumulate\"\nwhen = { per = \"event\", wrap = 4 }",
        );
        assert_eq!(
            run(&mut wrapped, &[events(3, 1.0), events(3, 1.0)]),
            [3.0, 2.0]
        );
        // It needs an event source, also as a fallback, and `strength` is only for it.
        let wrong = error("source = \"drive\"\nhow = \"accumulate\"\nwhen = { per = \"event\" }");
        assert!(
            wrong.contains("must count events") && wrong.contains("not `drive`"),
            "{wrong}"
        );
        assert!(error(
            "source = \"bars\"\nhow = \"accumulate\"\nwhen = { per = \"event\" }\nfallback = \"drive\""
        )
        .contains("must count events"));
        assert!(
            error("source = \"drive\"\nhow = \"accumulate\"\nwhen = { strength = 0.5 }")
                .contains("unknown key `strength`")
        );
        assert!(
            error("source = \"hits\"\nhow = \"accumulate\"\nwhen = { per = \"bar\" }")
                .contains("beat, second or event")
        );
    }

    #[test]
    fn an_accumulate_is_set_back_to_zero_by_an_event_source_such_as_the_scenes_cut() {
        let music = MusicValues { drive: 1.0, ..a() };
        let mut e = evaluator(
            "source = \"drive\"\nhow = \"accumulate\"\nwhen = { per = \"second\", reset = \"cut\" }",
        );
        let cut = Env {
            cut: true,
            ..env(&[], 0)
        };
        let mut tick = |env: &Env| e.tick_in(0.1, &music, Level::A, env).output;
        assert!(near(tick(&env(&[], 0)), 0.1) && near(tick(&env(&[], 0)), 0.2));
        // The sum is set back after this frame is added: it is 0 on the cut.
        assert!(near(tick(&cut), 0.0));
        assert!(near(tick(&env(&[], 0)), 0.1));
        // Any event source resets, from the music: here a bar line, on an event counter.
        let mut bars = evaluator(
            "source = \"hits\"\nhow = \"accumulate\"\nwhen = { per = \"event\", reset = \"bars\" }",
        );
        let out = run(
            &mut bars,
            &[
                hit(1.0),
                hit(1.0),
                MusicValues {
                    bars: 1,
                    ..hit(1.0)
                },
                hit(1.0),
            ],
        );
        assert_eq!(out, [1.0, 2.0, 0.0, 1.0]);
        // `cut` is a source of the scene: nothing in the music is a cut, so the runtime's own
        // bindings, which read the music alone, never see one.
        let source = sources::find("cut").unwrap();
        assert!(source.event && source.levels == [true; 3]);
        assert_eq!(source.read(&music).count, 0);
        assert_eq!(source.read_in(&music, true).count, 1);
        // A bind of it, and a whole scene's bindings told of the cut.
        let inputs = two_inputs();
        let binds = binds(&[
            "target = \"first\"\nsource = \"cut\"\nhow = \"accumulate\"\nwhen = { per = \"event\" }",
        ]);
        let mut bindings = Bindings::new(&inputs, &binds).unwrap();
        assert_eq!(bindings.tick_cut(0.1, &music, &[], 0, false)[0], 0.0);
        assert_eq!(bindings.tick_cut(0.1, &music, &[], 0, true)[0], 1.0);
        assert_eq!(bindings.tick(0.1, &music)[0], 1.0);
        // Only an event source resets.
        let wrong = error("source = \"drive\"\nhow = \"accumulate\"\nwhen = { reset = \"drive\" }");
        assert!(
            wrong.contains("`when.reset` must be a source that counts events"),
            "{wrong}"
        );
        assert!(
            error("source = \"drive\"\nhow = \"accumulate\"\nwhen = { reset = \"nope\" }")
                .contains("`when.reset`: no source `nope`")
        );
        assert!(
            error("source = \"drive\"\nhow = \"follow\"\nwhen = { reset = \"cut\" }")
                .contains("unknown key `reset`")
        );
    }

    #[test]
    fn a_number_may_be_a_small_expression_of_a_parameter() {
        let music = MusicValues { drive: 0.5, ..a() };
        let at = |text: &str, gain: f32| {
            let mut e = with_params(text).unwrap();
            e.tick_in(0.1, &music, Level::A, &env(&[gain, 3.0, 0.0], 0))
                .output
        };
        // `rate = param * 0.06`.
        let rate = "source = \"drive\"\nhow = \"accumulate\"\nwhen = { per = \"second\", rate = { param = \"gain\", mul = 0.06 } }";
        assert!(near(at(rate, 2.0), 0.5 * 0.12 * 0.1));
        assert!(near(at(rate, 1.0), 0.5 * 0.06 * 0.1));
        // `add` alone, and both: param * 2 + 1.
        let both = "source = \"drive\"\nhow = \"follow\"\nwhen = { to = [0, { param = \"gain\", mul = 2, add = 1 }] }";
        assert!(near(at(both, 2.0), 0.5 * 5.0));
        let add = "source = \"drive\"\nhow = \"follow\"\nwhen = { to = [{ param = \"gain\", add = 1 }, 10] }";
        assert!(near(at(add, 2.0), 3.0 + 0.5 * 7.0));
        // A `from` pair holds them too.
        let from = "source = \"drive\"\nhow = \"follow\"\nwhen = { from = [0, { param = \"gain\", mul = 2 }] }";
        assert!(near(at(from, 1.0), 0.25));
        assert!(near(at(from, 0.5), 0.5));
        // Anything else in the table is refused, naming the keys.
        let wrong = format!("{:#}", with_params("source = \"drive\"\nhow = \"follow\"\nwhen = { smooth = { param = \"gain\", scale = 2 } }").err().unwrap());
        assert!(
            wrong.contains("param = \"<id>\", mul = k, add = c"),
            "{wrong}"
        );
        let wrong = format!(
            "{:#}",
            with_params("source = \"drive\"\nhow = \"follow\"\nwhen = { smooth = { mul = 2 } }")
                .err()
                .unwrap()
        );
        assert!(wrong.contains("mul = k"), "{wrong}");
        let wrong = format!("{:#}", with_params("source = \"drive\"\nhow = \"follow\"\nwhen = { smooth = { param = \"gain\", mul = \"x\" } }").err().unwrap());
        assert!(
            wrong.contains("`mul` of the parameter must be a number"),
            "{wrong}"
        );
    }

    #[test]
    fn mul_may_be_a_number_as_well_as_a_source() {
        let music = MusicValues { drive: 0.5, ..a() };
        let mut plain =
            evaluator("source = \"drive\"\nhow = \"follow\"\nwhen = { mul = 0.5, add = 0.1 }");
        assert!(near(run(&mut plain, &[music])[0], 0.5 * 0.5 + 0.1));
        // A whole number too, and a parameter of the scene.
        let mut whole = evaluator("source = \"drive\"\nhow = \"follow\"\nwhen = { mul = 2 }");
        assert!(near(run(&mut whole, &[music])[0], 1.0));
        let mut param = with_params(
            "source = \"drive\"\nhow = \"follow\"\nwhen = { mul = { param = \"gain\", mul = 2 } }",
        )
        .unwrap();
        let t = param.tick_in(0.1, &music, Level::A, &env(&[0.5, 0.0, 0.0], 0));
        assert!(near(t.output, 0.5));
        // On an event it scales the strength.
        let mut strong = evaluator("source = \"hits\"\nhow = \"trigger\"\nwhen = { mul = 0.5 }");
        assert!(near(run(&mut strong, &[hit(0.8)])[0], 0.4));
        assert!(
            error("source = \"drive\"\nhow = \"follow\"\nwhen = { mul = true }")
                .contains("`when.mul` must be a number")
        );
    }

    #[test]
    fn a_follow_smooths_in_beats_or_in_seconds() {
        let low = MusicValues { drive: 0.0, ..a() };
        let high = MusicValues { drive: 1.0, ..a() };
        let step = |when: &str| {
            let mut e = evaluator(&format!(
                "source = \"drive\"\nhow = \"follow\"\nwhen = {{ smooth = 1.0{when} }}"
            ));
            run(&mut e, &[low, high])[1]
        };
        // At 120 bpm a frame of 0.1 s is 0.2 of the one-beat constant, and 0.1 of the second's.
        assert!(near(step(""), 1.0 - (-0.2f32).exp()));
        assert!(near(step(", per = \"beat\""), 1.0 - (-0.2f32).exp()));
        assert!(near(step(", per = \"second\""), 1.0 - (-0.1f32).exp()));
        // Without a followed beat a beat is 0.5 s, so seconds are what differs.
        let c = |drive| MusicValues {
            drive,
            ..at(Level::C)
        };
        let mut sec = evaluator(
            "source = \"drive\"\nhow = \"follow\"\nwhen = { smooth = 1.0, per = \"second\" }",
        );
        assert!(near(
            run(&mut sec, &[c(0.0), c(1.0)])[1],
            1.0 - (-0.1f32).exp()
        ));
        assert!(
            error("source = \"drive\"\nhow = \"follow\"\nwhen = { per = \"bar\" }")
                .contains("no `when.per`")
        );
    }

    #[test]
    fn a_pick_counts_only_events_within_a_band_of_strength() {
        let mut e = evaluator(
            "source = \"hits\"\nhow = \"pick\"\nwhen = { options = 4, strength = 0.3, max_strength = 0.6 }",
        );
        assert_eq!(
            run(&mut e, &[hit(0.2), hit(0.5), hit(0.8), hit(0.6), hit(0.3)]),
            [0.0, 1.0, 1.0, 2.0, 3.0]
        );
        // The most may be a parameter and only an event pick has one.
        assert!(error("source = \"section_label\"\nhow = \"pick\"\nwhen = { options = 3, max_strength = 0.5 }")
            .contains("`max_strength`"));
    }

    #[test]
    fn a_label_pick_can_start_from_the_seed_and_then_stay_put() {
        let label = |label: f32| MusicValues {
            section_label: label,
            ..a()
        };
        let pick = || {
            evaluator(
                "source = \"section_label\"\nhow = \"pick\"\nwhen = { options = 64, start = \"seed\" }",
            )
        };
        // One play: the first frame's seed salts it, and later seeds do not matter. The same
        // label always gets the same option, and a label that changes gets its own.
        let mut play = pick();
        let mut at = |l: f32, seed: u32| {
            play.tick_in(0.1, &label(l), Level::A, &env(&[], seed))
                .output as u32
        };
        let first: Vec<u32> = (0..6).map(|l| at(l as f32, 17 + l)).collect();
        let again: Vec<u32> = (0..6).map(|l| at(l as f32, 900 + l)).collect();
        assert_eq!(first, again);
        assert!(first.iter().all(|i| *i < 64));
        assert!(first.windows(2).any(|w| w[0] != w[1]));
        // Another play (another seed at load) may assign differently.
        let assign = |seed: u32| -> Vec<u32> {
            let mut e = pick();
            (0..8)
                .map(|l| {
                    e.tick_in(0.1, &label(l as f32), Level::A, &env(&[], seed))
                        .output as u32
                })
                .collect()
        };
        assert_eq!(assign(5), assign(5));
        assert_ne!(assign(5), assign(6));
        // Without `start` the assignment is the transitions' own, whatever the seed.
        let mut plain =
            evaluator("source = \"section_label\"\nhow = \"pick\"\nwhen = { options = 64 }");
        let out = plain
            .tick_in(0.1, &label(3.0), Level::A, &env(&[], 99))
            .output;
        assert_eq!(
            out as u32,
            pick_by_label(&(0..64).collect::<Vec<u32>>(), 3.0).unwrap()
        );
        // A start that is an index means nothing without events.
        assert!(
            error("source = \"section_label\"\nhow = \"pick\"\nwhen = { options = 3, start = 1 }")
                .contains("`start` is for a pick of an event source")
        );
    }
}
