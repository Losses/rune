//! The numbers a scene lab draws as time series: one catalogue of columns (what the director and
//! the looks are in, the music's signals, the scene's inputs), the live frames folded into
//! buckets of song time as they are drawn, and the same columns of a whole song as the plan
//! works them out (see [`crate::plan`]).
//!
//! Song time is cut into buckets of 1/[`RATE`] second. Within a bucket the frames (or the
//! steps of the plan) are folded by the kind of the column: a number keeps its maximum, so that
//! a short envelope is not lost between two samples; an event counts how many times it fired;
//! a bool, a category and a phase keep their last value. A value that is not there is NaN.

use std::collections::VecDeque;
use std::sync::Arc;

use crate::bind::{Bindings, Level, SOURCES, find};
use crate::inspect::Inspect;
use crate::scene::{Bind, FallbackDecl, InputDecl, LabRow, MusicValues, Param, STEP, mix_seed};

/// Buckets of song time in a second.
pub const RATE: u32 = 20;

/// The most finished buckets the live ring keeps: two minutes.
pub const RING: usize = 2400;

/// The steps of the plan folded into one bucket.
const STEPS_PER_BUCKET: u32 = 3;

/// How a column's frames are folded into a bucket.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnKind {
    /// The maximum.
    Number,
    /// The sum of the counts.
    Event,
    /// The last value, 0 or 1.
    Bool,
    /// The last value, a place in the column's categories.
    Category,
    /// The last value, a fraction from 0 to 1.
    Phase,
}

impl ColumnKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Number => "number",
            Self::Event => "event",
            Self::Bool => "bool",
            Self::Category => "category",
            Self::Phase => "phase",
        }
    }
}

/// Where a column is listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnGroup {
    Director,
    Looks,
    Signals,
    Inputs,
}

impl ColumnGroup {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Director => "Director",
            Self::Looks => "Looks & Level",
            Self::Signals => "Signals",
            Self::Inputs => "Inputs",
        }
    }
}

/// Whether the plan has the column's values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnPlan {
    Yes,
    /// It depends on what only the live audio says.
    No,
}

impl ColumnPlan {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Yes => "yes",
            Self::No => "no",
        }
    }
}

/// One column of the catalogue.
#[derive(Clone, Debug, PartialEq)]
pub struct LabColumn {
    pub id: String,
    pub name: String,
    pub group: ColumnGroup,
    pub kind: ColumnKind,
    pub lo: f32,
    pub hi: f32,
    /// Empty when the column has none.
    pub unit: String,
    /// What the places of a category stand for.
    pub categories: Vec<String>,
    pub plan: ColumnPlan,
    /// For a scene input, the ids of the columns its bindings read.
    pub binds: Vec<String>,
    /// For a scene input, the value it sits at when nothing drives it.
    pub rest: Option<f32>,
}

/// The columns of the scene on screen, in the order the values of a sample follow.
#[derive(Clone, Debug, PartialEq)]
pub struct LabColumns {
    /// Changes when the scene, the runtime's bindings, the scene's bindings or its parameters do,
    /// and with each new song.
    pub version: u64,
    pub columns: Vec<LabColumn>,
}

/// What the catalogue and the plan need of the scene on screen: its shots and looks, its inputs
/// and the bindings that drive them. Owned, so that the planner can keep it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LabScene {
    /// The name and the size ("wide", "medium", ...) of each shot, in the director's order.
    pub shots: Vec<(String, &'static str)>,
    pub looks: Vec<String>,
    pub inputs: Vec<InputDecl>,
    pub params: Vec<Param>,
    pub binds: Vec<Bind>,
    /// The numeric parameters as the bindings read them, in the manifest's order.
    pub param_values: Vec<f32>,
}

/// Places of the fixed columns.
pub const SHOT: usize = 0;
pub const SIZE: usize = 1;
pub const TIER: usize = 2;
pub const PACE: usize = 3;
pub const AGE: usize = 4;
pub const HELD: usize = 5;
pub const LOOK: usize = 6;
pub const MIX: usize = 7;
pub const LOOK_HELD: usize = 8;
pub const LEVEL: usize = 9;
/// Where the signals begin, one per source of the registry.
pub const SIGNALS: usize = 10;

/// The names of a shot's size, in the order the director numbers them.
const SIZES: [&str; 5] = ["wide", "medium", "close", "detail", "top"];
const TIERS: [&str; 3] = ["quiet", "normal", "high"];
/// The pace in effect, in the order of `Pace`.
const PACES: [&str; 4] = ["sections", "calm", "normal", "busy"];
const LEVELS: [&str; 3] = ["A", "B", "C"];

/// The sources that only the live audio says: a plan has nothing to read them from.
const LIVE_ONLY: [&str; 4] = ["band_low", "band_mid", "band_high", "flow"];

fn plannable(source: &str) -> bool {
    !LIVE_ONLY.contains(&source)
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(ToString::to_string).collect()
}

fn column(
    id: &str,
    name: &str,
    group: ColumnGroup,
    kind: ColumnKind,
    (lo, hi): (f32, f32),
    plan: ColumnPlan,
) -> LabColumn {
    LabColumn {
        id: id.to_string(),
        name: name.to_string(),
        group,
        kind,
        lo,
        hi,
        unit: String::new(),
        categories: Vec::new(),
        plan,
        binds: Vec::new(),
        rest: None,
    }
}

/// What a source's column is: its kind, range and unit.
fn signal_kind(name: &str, event: bool) -> (ColumnKind, (f32, f32), &'static str) {
    if event {
        return (ColumnKind::Event, (0.0, 4.0), "");
    }
    match name {
        "clock" | "flow" | "bar" | "beat" => (ColumnKind::Phase, (0.0, 1.0), ""),
        "locked" => (ColumnKind::Bool, (0.0, 1.0), ""),
        "band_low" | "band_mid" | "band_high" => (ColumnKind::Number, (0.0, 2.0), ""),
        "tempo" => (ColumnKind::Number, (60.0, 240.0), "bpm"),
        "meter" => (ColumnKind::Number, (2.0, 7.0), ""),
        "cue_in" => (ColumnKind::Number, (0.0, 16.0), "s"),
        "section" | "section_label" => (ColumnKind::Number, (0.0, 16.0), ""),
        _ => (ColumnKind::Number, (0.0, 1.0), ""),
    }
}

/// For each input of `scene`, whether the plan can work out its value: not when a binding that
/// drives it reads a source only the live audio says (or an input that is not plannable).
fn plannable_inputs(scene: &LabScene) -> Vec<bool> {
    let mut plannable_input = vec![true; scene.inputs.len()];
    let slot = |id: &str| scene.inputs.iter().position(|input| input.id == id);
    loop {
        let mut changed = false;
        for bind in &scene.binds {
            let Some(target) = slot(&bind.target).filter(|&t| plannable_input[t]) else {
                continue;
            };
            let fallback = match &bind.fallback {
                Some(FallbackDecl::Word(word)) => Some(word.as_str()),
                _ => None,
            };
            let reads_live = std::iter::once(bind.source.as_str())
                .chain(fallback)
                .any(|read| match read.strip_prefix("input:") {
                    Some(id) => slot(id).is_some_and(|input| !plannable_input[input]),
                    None => !plannable(read),
                });
            if reads_live {
                plannable_input[target] = false;
                changed = true;
            }
        }
        if !changed {
            return plannable_input;
        }
    }
}

impl LabColumns {
    /// The catalogue of `scene`, in `version`.
    pub fn of(scene: &LabScene, version: u64) -> Self {
        use ColumnGroup::{Director, Inputs, Looks, Signals};
        use ColumnKind::{Bool, Category, Number};
        use ColumnPlan::{No, Yes};
        let mut shot = column("shot", "Shot", Director, Category, (0.0, 0.0), Yes);
        shot.categories = scene
            .shots
            .iter()
            .enumerate()
            .map(|(index, (name, size))| {
                if name.is_empty() {
                    format!("#{} \u{b7} {size}", index + 1)
                } else {
                    format!("{name} \u{b7} {size}")
                }
            })
            .collect();
        let mut size = column("size", "Size", Director, Category, (0.0, 0.0), Yes);
        size.categories = names(&SIZES);
        let mut tier = column("tier", "Tier", Director, Category, (0.0, 0.0), Yes);
        tier.categories = names(&TIERS);
        let mut pace = column("pace", "Pace", Director, Category, (0.0, 0.0), Yes);
        pace.categories = names(&PACES);
        let mut age = column("age", "Age", Director, Number, (0.0, 8.0), Yes);
        age.unit = "s".to_string();
        let mut look = column("look", "Look", Looks, Category, (0.0, 0.0), Yes);
        look.categories = scene
            .looks
            .iter()
            .enumerate()
            .map(|(index, name)| {
                if name.is_empty() {
                    format!("#{}", index + 1)
                } else {
                    name.clone()
                }
            })
            .collect();
        let mut level = column("level", "Level", Looks, Category, (0.0, 0.0), Yes);
        level.categories = names(&LEVELS);
        let held = column("held", "Held", Director, Bool, (0.0, 1.0), Yes);
        let mix = column("mix", "Mix", Looks, Number, (0.0, 1.0), Yes);
        let look_held = column("look_held", "Look Held", Looks, Bool, (0.0, 1.0), Yes);
        let mut columns = vec![
            shot, size, tier, pace, age, held, look, mix, look_held, level,
        ];
        for source in SOURCES {
            let (kind, range, unit) = signal_kind(source.name, source.event);
            let live = !plannable(source.name);
            let mut signal = column(
                source.name,
                source.name,
                Signals,
                kind,
                range,
                if live { No } else { Yes },
            );
            signal.unit = unit.to_string();
            columns.push(signal);
        }
        let plannable_input = plannable_inputs(scene);
        for (index, input) in scene.inputs.iter().enumerate() {
            let label = input
                .label
                .as_ref()
                .map_or("", |label| label.get("en"))
                .to_string();
            let mut sources: Vec<&str> = Vec::new();
            for bind in scene.binds.iter().filter(|bind| bind.target == input.id) {
                let fallback = match &bind.fallback {
                    Some(FallbackDecl::Word(word)) if find(word).is_some() => Some(word),
                    _ => None,
                };
                for read in std::iter::once(&bind.source).chain(fallback) {
                    if !sources.contains(&read.as_str()) {
                        sources.push(read);
                    }
                }
            }
            let mut entry = column(
                &format!("input:{}", input.id),
                if label.is_empty() { &input.id } else { &label },
                Inputs,
                Number,
                (input.range[0], input.range[1]),
                if plannable_input[index] { Yes } else { No },
            );
            entry.rest = Some(input.rest());
            entry.binds = sources.iter().map(ToString::to_string).collect();
            columns.push(entry);
        }
        Self { version, columns }
    }

    /// Where the inputs begin.
    pub fn inputs_at() -> usize {
        SIGNALS + SOURCES.len()
    }

    fn kinds(&self) -> Vec<ColumnKind> {
        self.columns.iter().map(|column| column.kind).collect()
    }
}

/// Fold one frame's values into the bucket `acc` by the kinds of the columns. A value that is
/// not there (NaN) leaves the bucket as it is.
pub fn fold(kinds: &[ColumnKind], acc: &mut [f32], frame: &[f32]) {
    for ((kind, acc), &value) in kinds.iter().zip(acc).zip(frame) {
        if value.is_nan() {
            continue;
        }
        *acc = match kind {
            ColumnKind::Number if !acc.is_nan() => acc.max(value),
            ColumnKind::Event if !acc.is_nan() => *acc + value,
            _ => value,
        };
    }
}

/// Folds frames into the bucket of their song time and gives the buckets that are finished.
struct Folder {
    kinds: Vec<ColumnKind>,
    /// The bucket being folded, and its values so far.
    open: Option<u32>,
    values: Vec<f32>,
}

impl Folder {
    fn new(kinds: Vec<ColumnKind>) -> Self {
        let values = vec![f32::NAN; kinds.len()];
        Self {
            kinds,
            open: None,
            values,
        }
    }

    /// Add a frame to `bucket`. When that is not the bucket being folded (the song moved on, or
    /// jumped) the bucket being folded is finished and given.
    fn add(&mut self, bucket: u32, frame: &[f32]) -> Option<(u32, Vec<f32>)> {
        let finished = match self.open {
            Some(open) if open != bucket => self.take(),
            _ => None,
        };
        self.open = Some(bucket);
        fold(&self.kinds, &mut self.values, frame);
        finished
    }

    /// The bucket being folded, finished.
    fn take(&mut self) -> Option<(u32, Vec<f32>)> {
        let bucket = self.open.take()?;
        let fresh = vec![f32::NAN; self.kinds.len()];
        Some((bucket, std::mem::replace(&mut self.values, fresh)))
    }
}

/// A bucket of the live frames, finished.
#[derive(Clone, Debug, PartialEq)]
pub struct LabSample {
    pub bucket: u32,
    /// The catalogue the values follow.
    pub columns_version: u64,
    /// One per column, in the catalogue's order.
    pub values: Vec<f32>,
}

/// The bucket of a song time.
fn bucket_of(time: f64) -> u32 {
    (time.max(0.0) * f64::from(RATE)).floor() as u32
}

/// The place of `name` in `list` as a number, NaN if it is not there.
fn place(list: &[&str], name: &str) -> f32 {
    list.iter()
        .position(|item| *item == name)
        .map_or(f32::NAN, |place| place as f32)
}

/// The catalogue of the scene on screen and the live frames folded into samples, while a lab
/// inspects (see [`crate::Visualizer::lab_columns`]).
#[derive(Default)]
pub struct LabState {
    /// What the catalogue is of: the scene, the runtime's decisions, the scene's edits and the
    /// song's generation.
    key: Option<(u64, u64, u64, u64)>,
    columns: Option<Arc<LabColumns>>,
    scene: Option<Arc<LabScene>>,
    /// The catalogues made so far, so that each has a version of its own.
    versions: u64,
    folder: Option<Folder>,
    /// The values of the frame being folded.
    frame: Vec<f32>,
    ring: VecDeque<LabSample>,
}

impl LabState {
    /// Make the catalogue current for the scene and song `key` says (the three parts of
    /// `Pipeline::lab_key`, then the song's generation, see `Playback::generation`), built from
    /// `scene` only when the key changed. The bucket being folded under the old catalogue is
    /// finished first. A new song is a new catalogue, so that what the old one still had open is
    /// not taken for the new one's start.
    pub fn follow(&mut self, key: (u64, u64, u64, u64), scene: impl FnOnce() -> LabScene) {
        if self.key == Some(key) {
            return;
        }
        self.finish_bucket();
        self.key = Some(key);
        self.versions += 1;
        let scene = scene();
        let columns = LabColumns::of(&scene, self.versions);
        self.frame = vec![f32::NAN; columns.columns.len()];
        self.folder = Some(Folder::new(columns.kinds()));
        self.columns = Some(Arc::new(columns));
        self.scene = Some(Arc::new(scene));
    }

    /// The catalogue of the scene on screen.
    pub fn columns(&self) -> Option<Arc<LabColumns>> {
        self.columns.clone()
    }

    /// The scene the catalogue is of.
    pub fn scene(&self) -> Option<Arc<LabScene>> {
        self.scene.clone()
    }

    /// Fold the frame `inspect` says the render thread drew at song time `time`, with the music
    /// at `level`.
    pub fn record(&mut self, time: f64, inspect: &Inspect, level: Level) {
        let (Some(columns), Some(folder)) = (&self.columns, &mut self.folder) else {
            return;
        };
        let frame = &mut self.frame;
        frame.fill(f32::NAN);
        if let Some(director) = &inspect.director {
            frame[SHOT] = director.shot_index as f32;
            frame[SIZE] = place(&SIZES, &director.size);
            frame[TIER] = place(&TIERS, &director.tier);
            frame[PACE] = place(&PACES, &director.pace);
            frame[AGE] = director.age;
            frame[HELD] = f32::from(u8::from(director.held));
        }
        if let Some(looks) = &inspect.looks {
            frame[LOOK] = looks.on as f32;
            frame[MIX] = looks.mix;
            frame[LOOK_HELD] = f32::from(u8::from(looks.held));
        }
        frame[LEVEL] = level.index() as f32;
        for (at, signal) in inspect.signals.iter().enumerate().take(SOURCES.len()) {
            frame[SIGNALS + at] =
                signal_value(SOURCES[at].name, signal.event, signal.value, signal.count);
        }
        for (at, input) in inspect.inputs.iter().enumerate() {
            let value = inspect
                .bindings
                .iter()
                .rev()
                .find(|binding| binding.target == input.id)
                .map_or(input.rest, |binding| binding.output);
            if let Some(slot) = frame.get_mut(LabColumns::inputs_at() + at) {
                *slot = value;
            }
        }
        if let Some((bucket, values)) = folder.add(bucket_of(time), frame) {
            let version = columns.version;
            self.push(LabSample {
                bucket,
                columns_version: version,
                values,
            });
        }
    }

    fn finish_bucket(&mut self) {
        let version = self.columns.as_ref().map(|columns| columns.version);
        if let (Some(version), Some((bucket, values))) =
            (version, self.folder.as_mut().and_then(Folder::take))
        {
            self.push(LabSample {
                bucket,
                columns_version: version,
                values,
            });
        }
    }

    fn push(&mut self, sample: LabSample) {
        if self.ring.len() == RING {
            self.ring.pop_front();
        }
        self.ring.push_back(sample);
    }

    /// The buckets finished since the last call, oldest first.
    pub fn drain(&mut self) -> Vec<LabSample> {
        self.ring.drain(..).collect()
    }

    /// Forget everything, as when the lab goes off. The versions go on counting, so that a
    /// reader that kept one does not take a later catalogue for it.
    pub fn reset(&mut self) {
        *self = Self {
            versions: self.versions,
            ..Self::default()
        };
    }
}

/// What a source's column holds for a reading: the count of an event source, else the value
/// (`cue_in` has none, and says -1, without a chart).
fn signal_value(name: &str, event: bool, value: f32, count: u32) -> f32 {
    if event {
        count as f32
    } else if name == "cue_in" && value < 0.0 {
        f32::NAN
    } else if signal_kind(name, event).0 == ColumnKind::Phase {
        // A phase counts up through the song; its column is where in the cycle it is.
        value.rem_euclid(1.0)
    } else {
        value
    }
}

/// The columns of a song as the plan works them out, in the buckets of [`RATE`].
#[derive(Clone, Debug, Default)]
pub struct PlanSeries {
    /// The catalogue the values follow.
    pub columns_version: u64,
    /// How many buckets the song has.
    pub buckets: u32,
    /// Column-major: the value of column `c` in bucket `b` is at `c * buckets + b`. NaN where the
    /// plan has none.
    pub values: Vec<f32>,
}

/// NaN is the same as NaN here: two plans of the same song are the same.
impl PartialEq for PlanSeries {
    fn eq(&self, other: &Self) -> bool {
        self.columns_version == other.columns_version
            && self.buckets == other.buckets
            && self.values.len() == other.values.len()
            && self
                .values
                .iter()
                .zip(&other.values)
                .all(|(a, b)| a.to_bits() == b.to_bits())
    }
}

impl PlanSeries {
    /// The values of column `column`, one per bucket.
    pub fn column(&self, column: usize) -> &[f32] {
        let buckets = self.buckets as usize;
        &self.values[column * buckets..(column + 1) * buckets]
    }
}

/// What the planner needs to work the columns out: the catalogue and the scene it is of.
#[derive(Clone)]
pub struct PlanLab {
    pub columns: Arc<LabColumns>,
    pub scene: Arc<LabScene>,
}

/// Works the columns of a song out step by step: the steps the plan decides in, folded three to
/// a bucket.
pub struct SeriesRecorder {
    columns: Arc<LabColumns>,
    /// The scene's bindings, if any input can be planned.
    bindings: Option<Bindings>,
    params: Vec<f32>,
    seed: u32,
    folder: Folder,
    row: Vec<f32>,
    /// The finished buckets, one after the other.
    buckets: Vec<f32>,
}

impl SeriesRecorder {
    /// A recorder for a song with this `seed`.
    pub fn new(lab: &PlanLab, seed: u32) -> Self {
        let columns = Arc::clone(&lab.columns);
        let any_input = columns.columns[LabColumns::inputs_at()..]
            .iter()
            .any(|column| column.plan == ColumnPlan::Yes);
        let scene = &lab.scene;
        let bindings = any_input
            .then(|| Bindings::with_scope(&scene.inputs, &scene.params, &scene.binds).ok())
            .flatten();
        Self {
            row: vec![f32::NAN; columns.columns.len()],
            folder: Folder::new(columns.kinds()),
            bindings,
            params: scene.param_values.clone(),
            seed,
            buckets: Vec::new(),
            columns,
        }
    }

    /// Take step `step` of the song, in which the music was `music`, the decider was in `row`
    /// and the director did (`cut`) or did not cut.
    pub fn step(&mut self, step: u32, music: &MusicValues, row: &LabRow, cut: bool) {
        let at = &mut self.row;
        at.fill(f32::NAN);
        let number = |value: Option<u32>| value.map_or(f32::NAN, |value| value as f32);
        at[SHOT] = number(row.shot);
        at[SIZE] = number(row.size);
        at[TIER] = number(row.tier);
        at[PACE] = number(row.pace);
        at[AGE] = row.age.unwrap_or(f32::NAN);
        at[HELD] = row.held.map_or(f32::NAN, |held| f32::from(u8::from(held)));
        at[LOOK] = number(row.look);
        at[MIX] = row.mix.unwrap_or(f32::NAN);
        at[LOOK_HELD] = row
            .look_held
            .map_or(f32::NAN, |held| f32::from(u8::from(held)));
        at[LEVEL] = Level::of(music).index() as f32;
        for (source, slot) in SOURCES.iter().zip(&mut at[SIGNALS..]) {
            if plannable(source.name) {
                let reading = source.read_in(music, cut);
                *slot = signal_value(source.name, source.event, reading.value, reading.count);
            }
        }
        if let Some(bindings) = &mut self.bindings {
            let seed = mix_seed(self.seed, step);
            let values = bindings.tick_cut(STEP as f32, music, &self.params, seed, cut);
            for (slot, value) in at[LabColumns::inputs_at()..].iter_mut().zip(values) {
                *slot = *value;
            }
        }
        // What the plan does not follow is not there, whatever was worked out on the way.
        for (slot, column) in at.iter_mut().zip(&self.columns.columns) {
            if column.plan != ColumnPlan::Yes {
                *slot = f32::NAN;
            }
        }
        if let Some((_, values)) = self.folder.add(step / STEPS_PER_BUCKET, at) {
            self.buckets.extend(values);
        }
    }

    /// The series of the steps taken.
    pub fn finish(mut self) -> PlanSeries {
        if let Some((_, values)) = self.folder.take() {
            self.buckets.extend(values);
        }
        let width = self.columns.columns.len();
        let buckets = self.buckets.len() / width.max(1);
        let mut values = vec![f32::NAN; self.buckets.len()];
        for bucket in 0..buckets {
            for column in 0..width {
                values[column * buckets + bucket] = self.buckets[bucket * width + column];
            }
        }
        PlanSeries {
            columns_version: self.columns.version,
            buckets: buckets as u32,
            values,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::fixtures;

    const NAN: f32 = f32::NAN;

    fn same(a: &[f32], b: &[f32]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.to_bits() == b.to_bits())
    }

    #[test]
    fn a_bucket_keeps_the_maximum_of_numbers_the_sum_of_events_and_the_last_of_the_rest() {
        use ColumnKind::{Bool, Category, Event, Number, Phase};
        let kinds = [Number, Event, Bool, Category, Phase, Number];
        let mut folder = Folder::new(kinds.to_vec());
        assert!(folder.add(7, &[0.2, 1.0, 0.0, 1.0, 0.1, NAN]).is_none());
        assert!(folder.add(7, &[0.9, 0.0, 1.0, 2.0, 0.5, NAN]).is_none());
        assert!(folder.add(7, &[0.4, 2.0, 0.0, 0.0, 0.9, NAN]).is_none());
        let (bucket, values) = folder.add(8, &[0.1, 1.0, 1.0, 3.0, 0.0, 4.0]).unwrap();
        assert_eq!(bucket, 7);
        // A column that never had a value stays NaN.
        assert!(same(&values, &[0.9, 3.0, 0.0, 0.0, 0.9, NAN]), "{values:?}");
        // The next bucket starts from nothing, and a jump back finishes the one being folded.
        let (bucket, values) = folder.add(3, &[0.0; 6]).unwrap();
        assert_eq!(bucket, 8);
        assert!(same(&values, &[0.1, 1.0, 1.0, 3.0, 0.0, 4.0]), "{values:?}");
        let (bucket, _) = folder.take().unwrap();
        assert_eq!(bucket, 3);
        assert!(folder.take().is_none());
    }

    #[test]
    fn the_catalogue_lists_the_director_the_looks_the_signals_and_the_inputs_in_order() {
        let scene = fixtures::lab_scene();
        let columns = LabColumns::of(&scene, 5);
        assert_eq!(columns.version, 5);
        let head: Vec<_> = columns.columns[..SIGNALS]
            .iter()
            .map(|c| {
                (
                    c.id.as_str(),
                    c.name.as_str(),
                    c.group.as_str(),
                    c.kind.as_str(),
                )
            })
            .collect();
        assert_eq!(
            head,
            [
                ("shot", "Shot", "Director", "category"),
                ("size", "Size", "Director", "category"),
                ("tier", "Tier", "Director", "category"),
                ("pace", "Pace", "Director", "category"),
                ("age", "Age", "Director", "number"),
                ("held", "Held", "Director", "bool"),
                ("look", "Look", "Looks & Level", "category"),
                ("mix", "Mix", "Looks & Level", "number"),
                ("look_held", "Look Held", "Looks & Level", "bool"),
                ("level", "Level", "Looks & Level", "category"),
            ]
        );
        let plans: Vec<_> = columns.columns[..SIGNALS]
            .iter()
            .map(|c| c.plan.as_str())
            .collect();
        assert_eq!(
            plans,
            [
                "yes", "yes", "yes", "yes", "yes", "yes", "yes", "yes", "yes", "yes"
            ]
        );
        let signals = &columns.columns[SIGNALS..LabColumns::inputs_at()];
        assert!(
            signals
                .iter()
                .map(|c| c.id.as_str())
                .eq(SOURCES.iter().map(|s| s.name))
        );
        assert!(signals.iter().all(|c| c.group == ColumnGroup::Signals));
        let kind = |name: &str| signals.iter().find(|c| c.id == name).unwrap().kind;
        for name in [
            "hits",
            "bars",
            "beats",
            "cues",
            "impacts",
            "notes",
            "cut",
            "moment.strong",
            "moment.medium",
        ] {
            assert_eq!(kind(name), ColumnKind::Event, "{name}");
        }
        for name in ["clock", "flow", "bar", "beat"] {
            assert_eq!(kind(name), ColumnKind::Phase, "{name}");
        }
        assert_eq!(kind("locked"), ColumnKind::Bool);
        assert_eq!(kind("drive"), ColumnKind::Number);
        let live: Vec<_> = signals
            .iter()
            .filter(|c| c.plan == ColumnPlan::No)
            .map(|c| c.id.as_str())
            .collect();
        assert_eq!(live, ["flow", "band_low", "band_mid", "band_high"]);
        let range = |name: &str| {
            let c = signals.iter().find(|c| c.id == name).unwrap();
            (c.lo, c.hi, c.unit.as_str())
        };
        assert_eq!(range("tempo"), (60.0, 240.0, "bpm"));
        assert_eq!(range("cue_in"), (0.0, 16.0, "s"));
        assert_eq!(range("band_mid"), (0.0, 2.0, ""));
        let inputs = &columns.columns[LabColumns::inputs_at()..];
        assert!(
            inputs
                .iter()
                .map(|c| c.id.strip_prefix("input:").unwrap())
                .eq(scene.inputs.iter().map(|i| i.id.as_str()))
        );
        assert!(
            inputs
                .iter()
                .all(|c| c.group == ColumnGroup::Inputs && c.kind == ColumnKind::Number)
        );
        assert_eq!(
            columns.columns.len(),
            LabColumns::inputs_at() + scene.inputs.len()
        );
        // The shots and the looks of the scene are the categories.
        assert_eq!(columns.columns[SHOT].categories.len(), scene.shots.len());
        assert!(columns.columns[SHOT].categories[0].contains(" \u{b7} "));
        assert_eq!(columns.columns[LOOK].categories, scene.looks);
        assert_eq!(columns.columns[SIZE].categories[0], "wide");
        assert_eq!(
            columns.columns[TIER].categories,
            ["quiet", "normal", "high"]
        );
        assert_eq!(columns.columns[LEVEL].categories, ["A", "B", "C"]);
    }

    #[test]
    fn an_input_names_what_it_reads_and_the_other_columns_nothing() {
        let scene = fixtures::lab_scene_with(
            r#"
[[input]]
id = "a"
[[input]]
id = "b"
[[input]]
id = "c"
[[bind]]
target = "a"
source = "drive"
how = "follow"
fallback = "hit"
[[bind]]
target = "b"
source = "input:a"
how = "follow"
"#,
        );
        let columns = LabColumns::of(&scene, 1);
        for column in &columns.columns[..LabColumns::inputs_at()] {
            assert!(column.binds.is_empty(), "{}", column.id);
        }
        let inputs = &columns.columns[LabColumns::inputs_at()..];
        assert_eq!(inputs[0].binds, ["drive", "hit"]);
        assert_eq!(inputs[1].binds, ["input:a"]);
        assert!(inputs[2].binds.is_empty());
    }

    #[test]
    fn an_input_carries_the_value_it_rests_at_and_the_other_columns_none() {
        let scene = fixtures::lab_scene_with(
            r#"
[[input]]
id = "a"
range = [0.0, 2.0]
rest = 1.0
[[input]]
id = "b"
range = [0.5, 2.0]
"#,
        );
        let columns = LabColumns::of(&scene, 1);
        assert!(
            columns.columns[..LabColumns::inputs_at()]
                .iter()
                .all(|column| column.rest.is_none())
        );
        let inputs = &columns.columns[LabColumns::inputs_at()..];
        assert_eq!(inputs[0].rest, Some(1.0));
        // Without a rest, an input sits at the bottom of its range.
        assert_eq!(inputs[1].rest, Some(0.5));
    }

    #[test]
    fn an_input_is_planned_unless_a_binding_reads_what_only_the_live_audio_says() {
        let scene = fixtures::lab_scene_with(
            r#"
[[input]]
id = "a"
label = "Drive"
[[input]]
id = "b"
[[input]]
id = "c"
range = [0.0, 4.0]
[[input]]
id = "d"
[[bind]]
target = "a"
source = "drive"
how = "follow"
fallback = "hit"
[[bind]]
target = "b"
source = "band_low"
how = "follow"
[[bind]]
target = "c"
source = "input:b"
how = "follow"
"#,
        );
        let columns = LabColumns::of(&scene, 1);
        let inputs: Vec<_> = columns.columns[LabColumns::inputs_at()..]
            .iter()
            .map(|c| (c.id.as_str(), c.name.as_str(), c.plan.as_str(), c.hi))
            .collect();
        assert_eq!(
            inputs,
            [
                ("input:a", "Drive", "yes", 1.0),
                ("input:b", "b", "no", 1.0),
                ("input:c", "c", "no", 4.0),
                ("input:d", "d", "yes", 1.0),
            ]
        );
        let binds: Vec<_> = columns.columns[LabColumns::inputs_at()..]
            .iter()
            .map(|c| c.binds.join(", "))
            .collect();
        assert_eq!(binds, ["drive, hit", "band_low", "input:b", ""]);
    }

    fn inspect_at(count: u32) -> Inspect {
        Inspect {
            signals: SOURCES
                .iter()
                .map(|source| crate::inspect::InspectSignal {
                    name: source.name.to_string(),
                    event: source.event,
                    value: if source.name == "hits" { 1.0 } else { 0.0 },
                    count: if source.name == "hits" { count } else { 0 },
                    meaningful: true,
                })
                .collect(),
            ..Inspect::default()
        }
    }

    #[test]
    fn live_frames_are_folded_into_buckets_and_a_changed_scene_starts_a_new_catalogue() {
        let mut lab = LabState::default();
        assert!(lab.columns().is_none());
        lab.follow((1, 1, 0, 0), fixtures::lab_scene);
        let version = lab.columns().unwrap().version;
        let hits = SIGNALS + SOURCES.iter().position(|s| s.name == "hits").unwrap();
        // Three frames of the bucket 0 and one of the bucket 1: nothing is finished until the
        // song moves on.
        for (time, count) in [(0.01, 1), (0.02, 0), (0.04, 2)] {
            lab.record(time, &inspect_at(count), Level::A);
        }
        assert!(lab.drain().is_empty());
        lab.record(0.06, &inspect_at(1), Level::A);
        let [sample] = &lab.drain()[..] else {
            panic!("a bucket")
        };
        assert_eq!((sample.bucket, sample.columns_version), (0, version));
        assert_eq!(sample.values.len(), lab.columns().unwrap().columns.len());
        assert_eq!(sample.values[hits], 3.0);
        assert_eq!(sample.values[LEVEL], 0.0);
        // A seek starts a new bucket.
        lab.record(30.0, &inspect_at(0), Level::B);
        let [sample] = &lab.drain()[..] else {
            panic!("a bucket")
        };
        assert_eq!((sample.bucket, sample.values[hits]), (1, 1.0));
        // The same key keeps the catalogue; another finishes the open bucket under the old one.
        lab.follow((1, 1, 0, 0), fixtures::lab_scene);
        assert_eq!(lab.columns().unwrap().version, version);
        lab.follow((1, 1, 1, 0), fixtures::lab_scene);
        let [sample] = &lab.drain()[..] else {
            panic!("a bucket")
        };
        assert_eq!((sample.bucket, sample.columns_version), (600, version));
        assert_eq!(lab.columns().unwrap().version, version + 1);
        // Going off forgets it all but keeps counting versions.
        lab.reset();
        assert!(lab.columns().is_none() && lab.drain().is_empty());
        lab.follow((1, 1, 1, 0), fixtures::lab_scene);
        assert_eq!(lab.columns().unwrap().version, version + 2);
    }

    #[test]
    fn the_ring_keeps_the_newest_two_minutes() {
        let mut lab = LabState::default();
        lab.follow((1, 1, 0, 0), fixtures::lab_scene);
        for n in 0..(RING as u32 + 10) {
            lab.record(
                f64::from(n) / f64::from(RATE),
                &Inspect::default(),
                Level::C,
            );
        }
        let samples = lab.drain();
        assert_eq!(samples.len(), RING);
        assert_eq!(samples[0].bucket, 9);
    }
}
