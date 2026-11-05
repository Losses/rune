//! A song played through ahead of time: what the camera, the looks and the moments of the music
//! will do in it, for a scene lab to show before it happens.
//!
//! The plan is made by stepping a fresh [`Decider`] (see `scene::decider`) over the chart from
//! the start to the end of the song, one 60th of a second at a time, through the same step the
//! scene on screen takes. Everything it reads is the chart and the seed, so the same song and
//! settings plan the same thing each time; the live audio, which the chart replaces wherever it
//! has a value, is left out.

use crate::chart::Chart;
use crate::inspect::LabEventKind;
pub(crate) use crate::lab::PlanLab;
use crate::lab::{PlanSeries, SeriesRecorder};
use crate::rhythm::Rhythm;
use crate::scene::Decider;
use crate::scene::Levels;

/// The step of the plan, in seconds: the step the scene on screen decides a song with a chart in.
pub use crate::scene::STEP;

/// The quality the plan's lens is stepped with: the flare gives way only to a low quality, and
/// that changes what is drawn, not what is decided.
const QUALITY: f32 = 1.0;

/// One section of the song.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PlanSection {
    /// Seconds.
    pub start: f64,
    pub end: f64,
    pub label: String,
    pub energy: f32,
    /// How lively the director takes it to be, by the runtime's decisions: "quiet", "normal" or
    /// "high".
    pub tier: String,
}

/// A cut of the camera.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PlanCut {
    /// Song seconds.
    pub time: f64,
    pub shot: String,
    /// "wide", "medium", "close", "detail" or "top".
    pub size: String,
    /// 0 for a normal cut, else the position in a burst (1 to 4).
    pub fill: u32,
    /// The cut lands on a strong transition.
    pub strong: bool,
    /// The view jumps, rather than glides.
    pub hard: bool,
    /// The transition that dresses it, by name.
    pub dress: Option<String>,
    /// The scene's first shot, which it begins with.
    pub first: bool,
}

/// A change of look: the first one is the look the scene opens in.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PlanLook {
    pub time: f64,
    pub look: String,
}

/// A moment of the music that plays an effect.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PlanMoment {
    pub time: f64,
    /// "medium" or "strong".
    pub strength: String,
    /// What it plays, by name: a transition that dresses a cut, or a lens effect; none if the
    /// scene lists nothing that suits the moment.
    pub effect: Option<String>,
}

/// What a scene does in a song, as played through ahead of time (see the module's doc).
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct SongPlan {
    /// The song's file, and the scene (its name) the plan is of.
    pub path: String,
    pub scene: String,
    /// Seconds.
    pub length: f64,
    pub sections: Vec<PlanSection>,
    pub cuts: Vec<PlanCut>,
    pub looks: Vec<PlanLook>,
    pub moments: Vec<PlanMoment>,
    /// The columns of the lab's catalogue over the song, when the planner was given one.
    pub series: Option<PlanSeries>,
}

/// How many steps are taken between two looks at whether the plan is still wanted: a few
/// hundred keep the check cheap and a superseded plan short-lived.
const CHECK_EVERY: u32 = 256;

/// Play `chart` through `decider` (which must not have heard anything yet) from the start to the
/// end of the song with this `seed`. The file and the scene are left for the caller to say.
///
/// With a `lab`, the plan also has the series of the catalogue's columns (see [`crate::lab`]).
///
/// `wanted` is asked every few hundred steps; once it says no (the plan was superseded), the
/// song is not played further and there is no plan.
pub fn plan_song(
    chart: &Chart,
    decider: Decider,
    seed: u32,
    lab: Option<&PlanLab>,
    wanted: impl Fn() -> bool,
) -> Option<SongPlan> {
    play(
        chart,
        decider,
        seed,
        lab,
        |_| (Rhythm::default(), Levels::default()),
        wanted,
    )
}

/// [`plan_song`] with a live tracker and levels at each song time: what the chart leaves of the
/// live audio, which decides nothing in a song with a chart (a test says so).
fn play(
    chart: &Chart,
    mut decider: Decider,
    seed: u32,
    lab: Option<&PlanLab>,
    live: impl Fn(f64) -> (Rhythm, Levels),
    wanted: impl Fn() -> bool,
) -> Option<SongPlan> {
    let mut recorder = lab.map(|lab| SeriesRecorder::new(lab, seed));
    let length = chart.length();
    let levels: Vec<f32> = chart
        .at(0.0)
        .song
        .map_or_else(Vec::new, |song| song.iter().map(|s| s.level).collect());
    let mut plan = SongPlan {
        length,
        sections: chart
            .sections()
            .into_iter()
            .zip(&levels)
            .map(|(section, &level)| PlanSection {
                start: section.start,
                end: section.end,
                label: section.label,
                energy: section.energy,
                tier: decider.tier_at(level).to_string(),
            })
            .collect(),
        ..SongPlan::default()
    };
    for frame in 0..(length / STEP).ceil() as u32 {
        if frame % CHECK_EVERY == 0 && !wanted() {
            return None;
        }
        let time = f64::from(frame) * STEP;
        let (decision, music) = decider.step_heard(chart, frame, seed, live(time), None, QUALITY);
        if let Some(recorder) = &mut recorder {
            // As the scene on screen counts it: a cut, or the first frame of a scene with a
            // director.
            let cut = decision.cut.is_some() || (frame == 0 && decider.director().is_some());
            recorder.step(frame, &music, &decider.lab_row(&music), cut);
        }
        for event in decider.events(time, &decision, frame == 0) {
            match event.kind {
                LabEventKind::Cut {
                    shot,
                    size,
                    fill,
                    strong,
                    hard,
                    dress,
                    first,
                } => plan.cuts.push(PlanCut {
                    time: event.time,
                    shot,
                    size,
                    fill,
                    strong,
                    hard,
                    dress,
                    first,
                }),
                LabEventKind::Look { look } => plan.looks.push(PlanLook {
                    time: event.time,
                    look,
                }),
                LabEventKind::Moment { strength, effect } => plan.moments.push(PlanMoment {
                    time: event.time,
                    strength,
                    effect,
                }),
            }
        }
    }
    plan.series = recorder.map(SeriesRecorder::finish);
    Some(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::trusted_rhythm;
    use crate::lab::LabColumns;
    use crate::scene::{fixtures, song_seed};
    use std::sync::Arc;

    /// A song of a quiet section and a high one, which the director leads to with a burst.
    const SECTIONS: [(f64, f64, f64); 3] =
        [(0.0, 30.0, 0.2), (30.0, 70.0, 0.9), (70.0, 100.0, 0.3)];

    /// Noise in place of the live audio: kicks, energy and bands that change every frame.
    fn noisy(t: f64) -> (Rhythm, Levels) {
        let h = |k: f64| ((t * 1000.0 + k).sin() * 43758.5453).fract().abs();
        (
            Rhythm {
                bpm: 97.0,
                beat: t * 1.6,
                confidence: 0.9,
                kick: h(1.0),
                snare: h(2.0),
                hat: h(3.0),
                energy: 2.0 * h(4.0),
            },
            Levels {
                bass: (3.0 * h(5.0)) as f32,
                mids: (3.0 * h(6.0)) as f32,
                treble: (3.0 * h(7.0)) as f32,
                feel: 6.0,
                song_feel: 6.0,
            },
        )
    }

    fn planned(chart: &Chart, decider: Decider, seed: u32) -> SongPlan {
        plan_song(chart, decider, seed, None, || true).expect("a plan")
    }

    #[test]
    fn a_superseded_plan_stops_early() {
        let chart = fixtures::chart(&SECTIONS);
        let asked = std::cell::Cell::new(0);
        let plan = plan_song(&chart, fixtures::decider(), 1, None, || {
            asked.set(asked.get() + 1);
            asked.get() < 3
        });
        assert!(plan.is_none());
        assert_eq!(asked.get(), 3);
    }

    #[test]
    fn a_plan_is_the_same_each_time_whatever_the_live_audio_says() {
        let chart = fixtures::chart(&SECTIONS);
        let seed = song_seed("/music/a.flac");
        let plan = planned(&chart, fixtures::decider(), seed);
        assert!(
            !plan.cuts.is_empty() && !plan.moments.is_empty(),
            "{plan:?}"
        );
        assert_eq!(plan, planned(&chart, fixtures::decider(), seed));
        assert_eq!(
            plan,
            play(&chart, fixtures::decider(), seed, None, noisy, || true).unwrap()
        );
    }

    #[test]
    fn the_plan_describes_the_song() {
        let chart = fixtures::chart(&SECTIONS);
        let plan = planned(&chart, fixtures::decider(), 1);
        assert_eq!(plan.length, 100.0);
        let tiers: Vec<_> = plan.sections.iter().map(|s| s.tier.as_str()).collect();
        assert_eq!(tiers, ["quiet", "high", "normal"]);
        assert_eq!(
            plan.sections
                .iter()
                .map(|s| s.label.as_str())
                .collect::<Vec<_>>(),
            ["A", "B", "C"]
        );
        // The scene begins on its first shot, and the looks on their first look.
        assert!(plan.cuts[0].first && plan.cuts[0].time == 0.0);
        assert!(plan.cuts.iter().skip(1).all(|c| !c.first));
        assert_eq!(plan.looks[0].time, 0.0);
    }

    #[test]
    fn a_burst_comes_before_a_lifting_section_and_a_strong_cut_follows() {
        let chart = fixtures::chart(&SECTIONS);
        let plan = planned(&chart, fixtures::decider(), 1);
        let fills: Vec<(u32, bool)> = plan
            .cuts
            .iter()
            .filter(|cut| cut.time < 31.0 && cut.time > 20.0)
            .map(|cut| (cut.fill, cut.strong))
            .collect();
        let burst = fills
            .iter()
            .position(|&(fill, _)| fill == 1)
            .expect("a burst");
        assert_eq!(
            fills[burst..],
            [(1, false), (2, false), (3, false), (4, false), (0, true)]
        );
        let last = plan.cuts.iter().position(|cut| cut.fill == 4).unwrap();
        assert!(plan.cuts[last].hard && plan.cuts[last + 1].hard);
        assert!(
            (30.0..30.05).contains(&plan.cuts[last + 1].time),
            "{plan:?}"
        );
    }

    /// A decider stepped frame by frame at the plan's step (the music's values by the step, then
    /// the decision) gives the cuts, looks and moments the plan says, and so does the event log.
    #[test]
    fn a_decider_stepped_frame_by_frame_decides_what_the_plan_says() {
        let chart = fixtures::chart(&SECTIONS);
        let seed = song_seed("/music/a.flac");
        let plan = planned(&chart, fixtures::decider(), seed);
        let mut decider = fixtures::decider();
        let (mut cuts, mut looks, mut moments) = (Vec::new(), Vec::new(), Vec::new());
        for frame in 0..(plan.length / STEP).ceil() as u32 {
            let time = f64::from(frame) * STEP;
            let state = chart.at(time);
            let rhythm = trusted_rhythm(Rhythm::default(), Some(&state));
            let music = decider.hear(&rhythm, Some(&state), Levels::default(), (time, true), STEP);
            let decision = decider.step(STEP as f32, &music, Some(&state), seed, None, 0.5);
            for event in decider.events(time, &decision, frame == 0) {
                match event.kind {
                    LabEventKind::Cut {
                        shot, fill, strong, ..
                    } => {
                        cuts.push((event.time, shot, fill, strong));
                    }
                    LabEventKind::Look { look } => looks.push((event.time, look)),
                    LabEventKind::Moment { strength, effect } => {
                        moments.push((event.time, strength, effect));
                    }
                }
            }
        }
        let planned: Vec<_> = plan
            .cuts
            .iter()
            .map(|c| (c.time, c.shot.clone(), c.fill, c.strong))
            .collect();
        assert_eq!(cuts, planned);
        let planned: Vec<_> = plan
            .looks
            .iter()
            .map(|l| (l.time, l.look.clone()))
            .collect();
        assert_eq!(looks, planned);
        let planned: Vec<_> = plan
            .moments
            .iter()
            .map(|m| (m.time, m.strength.clone(), m.effect.clone()))
            .collect();
        assert_eq!(moments, planned);
    }

    #[test]
    fn a_plan_follows_what_a_lab_set() {
        let chart = fixtures::chart(&SECTIONS);
        let plain = planned(&chart, fixtures::decider(), 1);
        let mut lab = fixtures::decider();
        lab.director_mut()
            .unwrap()
            .set_pace_override(Some(crate::scene::Pace::Sections));
        let slow = planned(&chart, lab.fresh(&fixtures::manifest()), 1);
        assert!(slow.cuts.len() < plain.cuts.len(), "{slow:?} {plain:?}");
    }

    /// The music, the decider's rows and the cut flag of every step of a song, from a decider
    /// stepped frame by frame the way the scene on screen does.
    fn stepped(
        chart: &Chart,
        seed: u32,
        steps: u32,
    ) -> Vec<(crate::scene::MusicValues, crate::scene::LabRow, bool)> {
        let mut decider = fixtures::decider();
        (0..steps)
            .map(|frame| {
                let time = f64::from(frame) * STEP;
                let state = chart.at(time);
                let rhythm = trusted_rhythm(Rhythm::default(), Some(&state));
                let music =
                    decider.hear(&rhythm, Some(&state), Levels::default(), (time, true), STEP);
                let decision = decider.step(STEP as f32, &music, Some(&state), seed, None, 0.5);
                let cut = decision.cut.is_some() || (frame == 0 && decider.director().is_some());
                (music, decider.lab_row(&music), cut)
            })
            .collect()
    }

    fn lab_of(scene: crate::lab::LabScene) -> PlanLab {
        PlanLab {
            columns: Arc::new(LabColumns::of(&scene, 3)),
            scene: Arc::new(scene),
        }
    }

    fn planned_with(chart: &Chart, lab: &PlanLab, seed: u32) -> SongPlan {
        plan_song(chart, fixtures::decider(), seed, Some(lab), || true).expect("a plan")
    }

    /// The column `id` of the plan's series.
    fn series_of<'a>(plan: &'a SongPlan, lab: &PlanLab, id: &str) -> &'a [f32] {
        let at = lab.columns.columns.iter().position(|c| c.id == id).unwrap();
        plan.series.as_ref().unwrap().column(at)
    }

    /// What a column holds in each bucket if its values by step are folded the way a column of
    /// this kind is.
    fn buckets_of(steps: &[f32], fold: impl Fn(&[f32]) -> f32) -> Vec<f32> {
        steps.chunks(3).map(fold).collect()
    }

    fn max(values: &[f32]) -> f32 {
        values.iter().copied().fold(f32::NAN, f32::max)
    }

    #[test]
    fn the_series_has_a_bucket_for_every_three_steps_and_follows_the_catalogue() {
        let chart = fixtures::chart(&SECTIONS);
        let lab = lab_of(fixtures::lab_scene());
        let plan = planned_with(&chart, &lab, 1);
        let series = plan.series.as_ref().unwrap();
        let steps = (plan.length / STEP).ceil() as u32;
        assert_eq!(series.columns_version, 3);
        assert_eq!(series.buckets, steps.div_ceil(3));
        assert_eq!(
            series.values.len(),
            series.buckets as usize * lab.columns.columns.len()
        );
        // Without a catalogue the plan has no series, and the rest is the same.
        let plain = plan_song(&chart, fixtures::decider(), 1, None, || true).unwrap();
        assert!(plain.series.is_none());
        assert_eq!(plain.cuts, plan.cuts);
    }

    #[test]
    fn the_director_and_look_columns_are_what_a_decider_stepped_frame_by_frame_is_in() {
        let chart = fixtures::chart(&SECTIONS);
        let seed = song_seed("/music/a.flac");
        let lab = lab_of(fixtures::lab_scene());
        let plan = planned_with(&chart, &lab, seed);
        let steps = (plan.length / STEP).ceil() as u32;
        let rows = stepped(&chart, seed, steps);
        // Last of each three steps.
        let last = |values: &[f32]| *values.last().unwrap();
        let by = |pick: &dyn Fn(&crate::scene::LabRow) -> f32| {
            let steps: Vec<f32> = rows.iter().map(|(_, row, _)| pick(row)).collect();
            buckets_of(&steps, last)
        };
        let same = |a: &[f32], b: &[f32]| a == b;
        assert!(same(
            series_of(&plan, &lab, "shot"),
            &by(&|r| r.shot.unwrap() as f32)
        ));
        assert!(same(
            series_of(&plan, &lab, "size"),
            &by(&|r| r.size.unwrap() as f32)
        ));
        assert!(same(
            series_of(&plan, &lab, "tier"),
            &by(&|r| r.tier.unwrap() as f32)
        ));
        assert!(same(
            series_of(&plan, &lab, "pace"),
            &by(&|r| r.pace.unwrap() as f32)
        ));
        assert!(same(
            series_of(&plan, &lab, "look"),
            &by(&|r| r.look.unwrap() as f32)
        ));
        let levels: Vec<f32> = rows
            .iter()
            .map(|(music, ..)| crate::bind::Level::of(music).index() as f32)
            .collect();
        assert!(same(
            series_of(&plan, &lab, "level"),
            &buckets_of(&levels, last)
        ));
        // The shots change over the song, and the series says when the plan's cuts came.
        let shots = series_of(&plan, &lab, "shot");
        assert!(shots.windows(2).any(|w| w[0] != w[1]), "{shots:?}");
        // Age and mix are numbers (the most of the three steps), held and look held are bools
        // (the last).
        let age: Vec<f32> = rows.iter().map(|(_, r, _)| r.age.unwrap()).collect();
        let mix: Vec<f32> = rows.iter().map(|(_, r, _)| r.mix.unwrap()).collect();
        assert!(same(series_of(&plan, &lab, "age"), &buckets_of(&age, max)));
        assert!(same(series_of(&plan, &lab, "mix"), &buckets_of(&mix, max)));
        assert!(
            series_of(&plan, &lab, "age").iter().any(|v| *v > 1.0),
            "the shot ages between cuts"
        );
        assert!(same(
            series_of(&plan, &lab, "held"),
            &by(&|r| f32::from(u8::from(r.held.unwrap())))
        ));
        assert!(same(
            series_of(&plan, &lab, "look_held"),
            &by(&|r| f32::from(u8::from(r.look_held.unwrap())))
        ));
    }

    #[test]
    fn a_held_shot_and_a_held_look_are_held_all_song_in_the_plan() {
        let chart = fixtures::chart(&SECTIONS);
        let lab = lab_of(fixtures::lab_scene());
        let free = planned_with(&chart, &lab, 1);
        for id in ["held", "look_held"] {
            assert!(series_of(&free, &lab, id).iter().all(|v| *v == 0.0), "{id}");
        }
        let mut decider = fixtures::decider();
        assert!(decider.hold_shot(1, &crate::scene::MusicValues::default()));
        assert!(decider.hold_look(1, &crate::scene::MusicValues::default()));
        let plan = plan_song(&chart, decider, 1, Some(&lab), || true).unwrap();
        for id in ["held", "look_held"] {
            assert!(series_of(&plan, &lab, id).iter().all(|v| *v == 1.0), "{id}");
        }
    }

    #[test]
    fn a_plannable_source_is_what_the_decider_heard_from_the_chart() {
        let chart = fixtures::chart(&SECTIONS);
        let seed = 9;
        let lab = lab_of(fixtures::lab_scene());
        let plan = planned_with(&chart, &lab, seed);
        let steps = (plan.length / STEP).ceil() as u32;
        let rows = stepped(&chart, seed, steps);
        let mut checked = 0;
        for source in crate::bind::SOURCES {
            let column = lab
                .columns
                .columns
                .iter()
                .find(|c| c.id == source.name && c.plan == crate::lab::ColumnPlan::Yes);
            let Some(column) = column else { continue };
            let values: Vec<f32> = rows
                .iter()
                .map(|(music, _, cut)| {
                    let reading = source.read_in(music, *cut);
                    match (column.kind, source.name) {
                        (crate::lab::ColumnKind::Event, _) => reading.count as f32,
                        (_, "cue_in") if reading.value < 0.0 => f32::NAN,
                        (crate::lab::ColumnKind::Phase, _) => reading.value.rem_euclid(1.0),
                        _ => reading.value,
                    }
                })
                .collect();
            let expected = match column.kind {
                crate::lab::ColumnKind::Number => buckets_of(&values, max),
                crate::lab::ColumnKind::Event => buckets_of(&values, |v| v.iter().sum()),
                _ => buckets_of(&values, |v| *v.last().unwrap()),
            };
            let got = series_of(&plan, &lab, &column.id);
            assert!(
                got.iter()
                    .zip(&expected)
                    .all(|(a, b)| a.to_bits() == b.to_bits()),
                "{}",
                source.name
            );
            checked += 1;
        }
        assert_eq!(checked, crate::bind::SOURCES.len() - 4);
        // The song has beats, so some of the events fired.
        assert!(series_of(&plan, &lab, "beats").iter().sum::<f32>() > 100.0);
    }

    #[test]
    fn an_input_bound_to_chart_sources_is_what_the_live_bindings_make_of_it() {
        let scene = fixtures::lab_scene_with(
            r#"
[[input]]
id = "energy"
[[input]]
id = "kick"
range = [0.0, 2.0]
[[input]]
id = "live"
[[bind]]
target = "energy"
source = "drive"
how = "follow"
[[bind]]
target = "kick"
source = "hits"
how = "trigger"
when = { release = 0.3 }
[[bind]]
target = "live"
source = "band_low"
how = "follow"
"#,
        );
        let chart = fixtures::chart(&SECTIONS);
        let seed = song_seed("/music/a.flac");
        let lab = lab_of(scene.clone());
        let plan = planned_with(&chart, &lab, seed);
        let steps = (plan.length / STEP).ceil() as u32;
        let rows = stepped(&chart, seed, steps);
        let mut bindings =
            crate::bind::Bindings::with_scope(&scene.inputs, &scene.params, &scene.binds).unwrap();
        let outputs: Vec<Vec<f32>> = rows
            .iter()
            .enumerate()
            .map(|(frame, (music, _, cut))| {
                let seed = crate::scene::mix_seed(seed, frame as u32);
                bindings
                    .tick_cut(STEP as f32, music, &scene.param_values, seed, *cut)
                    .to_vec()
            })
            .collect();
        for (slot, id) in ["input:energy", "input:kick"].into_iter().enumerate() {
            let values: Vec<f32> = outputs.iter().map(|o| o[slot]).collect();
            let expected = buckets_of(&values, max);
            assert_eq!(series_of(&plan, &lab, id), expected, "{id}");
        }
        assert!(
            series_of(&plan, &lab, "input:kick")
                .iter()
                .any(|v| *v > 0.5)
        );
        assert!(
            series_of(&plan, &lab, "input:live")
                .iter()
                .all(|v| v.is_nan())
        );
    }

    #[test]
    fn the_sources_only_the_live_audio_says_are_not_in_the_plan() {
        let chart = fixtures::chart(&SECTIONS);
        let lab = lab_of(fixtures::lab_scene());
        // Noise for the live audio changes nothing of the series either.
        let plan = planned_with(&chart, &lab, 1);
        let noisy = play(&chart, fixtures::decider(), 1, Some(&lab), noisy, || true).unwrap();
        assert_eq!(plan, noisy);
        for id in ["band_low", "band_mid", "band_high", "flow"] {
            assert!(
                series_of(&plan, &lab, id).iter().all(|v| v.is_nan()),
                "{id}"
            );
        }
        assert!(series_of(&plan, &lab, "drive").iter().any(|v| !v.is_nan()));
    }

    #[test]
    fn a_superseded_plan_with_series_stops_early() {
        let chart = fixtures::chart(&SECTIONS);
        let lab = lab_of(fixtures::lab_scene());
        let asked = std::cell::Cell::new(0);
        let plan = plan_song(&chart, fixtures::decider(), 1, Some(&lab), || {
            asked.set(asked.get() + 1);
            asked.get() < 3
        });
        assert!(plan.is_none());
        assert_eq!(asked.get(), 3);
    }

    /// Prints how long a plan of five minutes takes with and without the series:
    /// `cargo test -p visualizer planning_a_five_minute -- --nocapture`.
    #[test]
    fn planning_a_five_minute_song_with_series_is_quick() {
        let chart = fixtures::chart(&[(0.0, 100.0, 0.2), (100.0, 200.0, 0.9), (200.0, 300.0, 0.3)]);
        let lab = lab_of(fixtures::lab_scene());
        let time = |lab: Option<&PlanLab>| {
            let start = std::time::Instant::now();
            let plan = plan_song(&chart, fixtures::decider(), 1, lab, || true).unwrap();
            (start.elapsed(), plan)
        };
        let (without, _) = time(None);
        let (with, plan) = time(Some(&lab));
        eprintln!("plan of 300 s: {without:.2?} without series, {with:.2?} with");
        assert_eq!(plan.series.unwrap().buckets, 6000);
    }
}
