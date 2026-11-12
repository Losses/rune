//! What a scene's music decides, in one place: the camera director, the looks, the moments that
//! play an effect and the lens effects they start, with the counters and signals the music's
//! values are made with. It is stepped with an explicit time step and seed, and every part of it
//! is plain data, so it can be cloned: a song can be played through silently up to any moment
//! with the same steps (see `scene::stepper`), and what the screen draws is only what the
//! decisions say. The scene's own bindings are not in it; they hear the cut.
//!
//! One frame is [`Decider::hear`] (the music's values, by the wall time the caller says went by)
//! and [`Decider::step`] (the decisions), in the order the scene's shaders have always seen them:
//! the director, the looks, the moments, the lens. A song with a chart is decided by
//! [`Decider::step_at`] instead, in steps of [`STEP`] of song time heard from the chart alone, so
//! that the same song decides the same things at the same times however it is drawn (see
//! `scene::stepper`).

use super::CutInfo;
use super::Tier;
use super::director::Director;
use super::inputs::{self, EventCounter, Levels, MusicValues};
use super::lens::{Lens, LensFrame};
use super::looks::Looks;
use super::manifest::{Manifest, Pace, ShotMove, ShotSize};
use crate::bind::RuntimeBindings;
use crate::chart::{Chart, ChartState, trusted_rhythm};
use crate::inspect::{LabEvent, LabEventKind, LabLens};
use crate::moments::{self, Chances, Played, Scheduler, Strength};
use crate::rhythm::Rhythm;
use crate::transition::{Overlay, Transition};

/// The step a song with a chart is decided in, in seconds: a frame at 60 fps.
pub const STEP: f64 = 1.0 / 60.0;

/// What a moment that played did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Effect {
    /// It dressed the cut of the frame with a transition.
    Dressed(Transition),
    /// What a lab forced on started.
    Forced,
    /// The lens effect of the scene's list that suits it, if the list has one.
    Lens(Option<moments::Lens>),
}

/// A moment of the music that played an effect in a frame, and what it did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Moment {
    pub strength: Strength,
    pub effect: Effect,
}

/// What was decided in one frame: all the scene does with it is draw.
#[derive(Clone, Debug, PartialEq)]
pub struct Decision {
    /// The cut of the director (or a lab's), as the transition layer dresses it.
    pub cut: Option<CutInfo>,
    /// The shot the director is on after the frame; 0 without a director.
    pub shot: u32,
    /// The look the frame changed to; the first frame reports the look the scene opens in.
    pub look: Option<u32>,
    pub moment: Option<Moment>,
    /// What the post pass does in the frame, and the glitch or mosaic over the picture that
    /// starts with it.
    pub lens: LensFrame,
    pub overlay: Option<Overlay>,
}

/// What a lab or the user may set on a [`Decider`], which a plan of a song is made with (see
/// [`Decider::fresh`]) and which changes it when it changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub chances: Chances,
    pub pace: Option<Pace>,
    pub bursts: Option<bool>,
    pub shot: Option<u32>,
    pub look: Option<u32>,
    pub lens: LabLens,
}

/// What the director and the looks are in (see [`Decider::lab_row`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LabRow {
    pub shot: Option<u32>,
    pub size: Option<u32>,
    pub tier: Option<u32>,
    pub pace: Option<u32>,
    pub age: Option<f32>,
    pub held: Option<bool>,
    pub look: Option<u32>,
    pub mix: Option<f32>,
    pub look_held: Option<bool>,
}

/// The decisions of one scene.
#[derive(Clone)]
pub struct Decider {
    /// Decides the camera cuts, if the scene declares shots.
    director: Option<Director>,
    /// Decides the look changes, if the scene declares looks.
    looks: Option<Looks>,
    /// The look the last frame reported, so that a change is told once.
    look_on: Option<u32>,
    /// The moments of the music that play an effect, and their chances (see [`crate::moments`]).
    moments: Scheduler,
    chances: Chances,
    /// The lens effects the scene listed (`effects.lens`) and when each is on.
    lens: Lens,
    /// The transitions that may dress a cut of the director, by name (its `effects.cut`).
    cut_transitions: Vec<String>,
    /// The counters and signals the music's values are made with.
    events: EventCounter,
    /// The runtime's decisions as they were set, to make another decider the same way.
    runtime: Option<RuntimeBindings>,
    /// The shot and the look a lab holds, kept to hold them again.
    held_shot: Option<u32>,
    held_look: Option<u32>,
}

impl Decider {
    /// The decisions of a scene of this manifest, reading the embedded `runtime.toml` until told
    /// otherwise.
    pub fn new(manifest: &Manifest) -> Self {
        Self {
            director: Director::for_manifest(manifest),
            looks: Looks::for_manifest(manifest),
            look_on: None,
            moments: Scheduler::default(),
            chances: Chances::default(),
            lens: Lens::new(&manifest.effects.lens),
            cut_transitions: manifest.effects.cut.clone(),
            events: EventCounter::default(),
            runtime: None,
            held_shot: None,
            held_look: None,
        }
    }

    /// A decider of the same scene set the same way (the runtime's decisions, the chances, what
    /// a lab holds, forces and overrides) but from the start of a song, with nothing heard yet.
    pub fn fresh(&self, manifest: &Manifest) -> Self {
        let mut fresh = Self::new(manifest);
        if let Some(runtime) = &self.runtime {
            fresh.set_runtime(runtime);
        }
        fresh.chances = self.chances;
        fresh.lens.set_forced(self.lens.forced());
        if let (Some(old), Some(new)) = (&self.director, &mut fresh.director) {
            new.set_pace_override(old.pace_override());
            new.set_burst_override(old.burst_override());
        }
        // Held before anything is heard, a look is the one the scene opens in.
        let nothing = MusicValues::default();
        if let Some(index) = self.held_shot {
            fresh.hold_shot(index, &nothing);
        }
        if let Some(index) = self.held_look {
            fresh.hold_look(index, &nothing);
        }
        fresh
    }

    /// What a lab or the user set on the decider.
    pub fn settings(&self) -> Settings {
        Settings {
            chances: self.chances,
            pace: self.director.as_ref().and_then(Director::pace_override),
            bursts: self.director.as_ref().and_then(Director::burst_override),
            shot: self.held_shot,
            look: self.held_look,
            lens: self.lens.forced(),
        }
    }

    /// Decide by these runtime decisions from now on (`runtime.toml`).
    pub fn set_runtime(&mut self, runtime: &RuntimeBindings) {
        self.moments.set_binds(runtime.moments.clone());
        if let Some(director) = &mut self.director {
            director.set_binds(runtime.director.clone(), runtime.moments.clone());
        }
        if let Some(looks) = &mut self.looks {
            looks.set_binds(runtime.looks.clone(), runtime.director.clone());
        }
        self.lens.set_binds(runtime);
        self.runtime = Some(runtime.clone());
    }

    pub fn set_chances(&mut self, chances: Chances) {
        self.chances = chances;
    }

    pub fn director(&self) -> Option<&Director> {
        self.director.as_ref()
    }

    pub fn director_mut(&mut self) -> Option<&mut Director> {
        self.director.as_mut()
    }

    pub fn looks(&self) -> Option<&Looks> {
        self.looks.as_ref()
    }

    pub fn lens(&self) -> &Lens {
        &self.lens
    }

    pub fn lens_mut(&mut self) -> &mut Lens {
        &mut self.lens
    }

    pub fn cut_transitions(&self) -> &[String] {
        &self.cut_transitions
    }

    /// Cut to shot `index` and hold it, or let go of a held shot (see `Director::hold_shot`).
    /// Whether the scene has the shot, or a director at all.
    pub fn hold_shot(&mut self, index: u32, music: &MusicValues) -> bool {
        let held = self
            .director
            .as_mut()
            .is_some_and(|director| director.hold_shot(index, music).is_some());
        if held {
            self.held_shot = Some(index);
        }
        held
    }

    /// Let go of the held shot.
    pub fn release_shot(&mut self) {
        if let Some(director) = &mut self.director {
            director.release_shot();
        }
        self.held_shot = None;
    }

    /// Change to look `index` and hold it (see `Looks::hold`). Whether the scene has the look.
    pub fn hold_look(&mut self, index: u32, music: &MusicValues) -> bool {
        let held = self
            .looks
            .as_mut()
            .is_some_and(|looks| looks.hold(index, music));
        if held {
            self.held_look = Some(index);
        }
        held
    }

    /// Hand the look back to the runtime.
    pub fn release_look(&mut self) {
        if let Some(looks) = &mut self.looks {
            looks.release();
        }
        self.held_look = None;
    }

    /// Take over what is under way in `old`, the decider of the scene this one replaces when its
    /// files were saved: what a lab set (a hold of a shot is held again by the scene, which says
    /// what the music is), the look on with its fade, the chances.
    pub fn inherit(&mut self, old: &Self) {
        if let (Some(old), Some(new)) = (&old.director, &mut self.director) {
            new.set_pace_override(old.pace_override());
            new.set_burst_override(old.burst_override());
        }
        // The look on and a held one carry over only to the same looks.
        if let (Some(old_looks), Some(new)) = (&old.looks, &mut self.looks)
            && new.inherit(old_looks)
        {
            self.held_look = old.held_look;
            self.look_on = old.look_on;
        }
        self.lens.set_forced(old.lens.forced());
        self.chances = old.chances;
    }

    /// The music's values for a frame, by the wall seconds `wall_dt` since the last one.
    pub fn hear(
        &mut self,
        rhythm: &Rhythm,
        chart: Option<&ChartState>,
        levels: Levels,
        song: (f64, bool),
        wall_dt: f64,
    ) -> MusicValues {
        inputs::music(rhythm, chart, levels, song, &mut self.events, wall_dt)
    }

    /// Decide step `step` of a song with this whole `chart`: the frame at `step` times [`STEP`]
    /// of song time, with the music's values heard from the chart at that time and from the live
    /// audio (`live`) only where the chart has no value, which decides nothing then (see
    /// `crate::plan`'s tests). The same steps with the same `seed` decide the same things.
    pub fn step_at(
        &mut self,
        chart: &Chart,
        step: u32,
        seed: u32,
        (tracker, levels): (Rhythm, Levels),
        forced: Option<CutInfo>,
        quality: f32,
    ) -> Decision {
        self.step_heard(chart, step, seed, (tracker, levels), forced, quality)
            .0
    }

    /// As [`Decider::step_at`], with the music's values the step heard as well.
    pub fn step_heard(
        &mut self,
        chart: &Chart,
        step: u32,
        seed: u32,
        (tracker, levels): (Rhythm, Levels),
        forced: Option<CutInfo>,
        quality: f32,
    ) -> (Decision, MusicValues) {
        let time = f64::from(step) * STEP;
        let state = chart.at(time);
        let rhythm = trusted_rhythm(tracker, Some(&state));
        let music = self.hear(&rhythm, Some(&state), levels, (time, true), STEP);
        let decision = self.step(STEP as f32, &music, Some(&state), seed, forced, quality);
        (decision, music)
    }

    /// Decide a frame of `dt` seconds with its music and the frame's `seed`. `forced` is a cut a
    /// lab made since the last frame, reported when the director made none. `quality` is the one
    /// the scene draws with, which the flare gives way to.
    ///
    /// The scene's bindings hear the cut but nothing else of what is decided, and nothing here
    /// reads them, so they come after the whole step.
    pub fn step(
        &mut self,
        dt: f32,
        music: &MusicValues,
        chart: Option<&ChartState>,
        seed: u32,
        forced: Option<CutInfo>,
        quality: f32,
    ) -> Decision {
        // The director decides first, so that the cut can be dressed by the moment of the frame.
        let mut cut = None;
        let mut shot = 0;
        if let Some(director) = &mut self.director {
            director.set_song_ahead(chart.map(|c| c.later_rise as f32));
            director.set_song(chart.and_then(|c| c.song.as_ref()));
            cut = director
                .tick(dt, music)
                .map(|cut| CutInfo {
                    size: cut.size,
                    motion: cut.motion,
                    cue: music.cue,
                    fill: cut.fill,
                    dress: None,
                    strong: cut.strong,
                })
                .or(forced);
            shot = director.state().index;
        }
        let mut look = None;
        if let Some(looks) = &mut self.looks {
            looks.set_song(chart.and_then(|c| c.song.as_ref()));
            looks.tick(dt, music, seed);
            let on = looks.state().to;
            if self.look_on != Some(on) {
                self.look_on = Some(on);
                look = Some(on);
            }
        }
        // The lens effects hear where the music is in its phrase before a moment starts one.
        self.lens.listen(music);
        // A moment that plays an effect: a hard cut in it is dressed with a transition of the
        // scene's list; with no cut, a glide, or no transition that suits, a lens effect plays
        // over the picture.
        let mut moment = None;
        if let Some(played) = self.moments.tick(music, self.chances) {
            // What a lab forced on plays at every moment, on top of what the moment does.
            let forced = self.lens.start_forced(played);
            // A cut the scene glides to is not dressed: the moment plays a light lens effect
            // instead (no glitch or mosaic over a smooth move).
            let glides = cut
                .as_ref()
                .is_some_and(|cut| cut.motion == ShotMove::Glide);
            let dress = cut
                .as_ref()
                .and_then(|_| moments::dressing_of_cut(&self.cut_transitions, played, glides));
            let effect = match (&mut cut, dress) {
                (Some(cut), Some(kind)) => {
                    cut.dress = Some(kind);
                    Effect::Dressed(kind)
                }
                _ if forced => Effect::Forced,
                _ if glides => Effect::Lens(self.lens.start_over_glide(played)),
                _ => Effect::Lens(self.lens.start_at(played)),
            };
            moment = Some(Moment {
                strength: played.strength,
                effect,
            });
        }
        // The cut a burst leads to, and the start of a high section, land on a strong transition of
        // the scene's list, whatever the roll: four hard cuts ending on a plain one stopped short,
        // and a climax is always entered on a cut. A light one the moment gave it (a high section
        // after a high one does not lift, so its moment is a medium one) makes way for it.
        if let Some(cut) = &mut cut
            && cut.strong
            && !cut.dress.is_some_and(Transition::is_strong)
        {
            let played = Played {
                strength: Strength::Strong,
                label: music.section_label,
            };
            if let Some(kind) = moments::cut_dressing(&self.cut_transitions, played) {
                cut.dress = Some(kind);
            }
        }
        let lens = self.lens.tick(dt, music, quality);
        let overlay = self.lens.take_overlay();
        Decision {
            cut,
            shot,
            look,
            moment,
            lens,
            overlay,
        }
    }

    /// What the director and the looks are in after a step that heard `music`, as numbers (places
    /// in the lists of names): none where the scene has no director or no looks.
    pub fn lab_row(&self, music: &MusicValues) -> LabRow {
        let mut row = LabRow::default();
        if let Some(director) = &self.director {
            let state = director.state();
            row.shot = Some(state.index);
            row.size = Some(state.size);
            row.tier = Some(director.tier_index(music) as u32);
            row.pace = Some(director.pace() as u32);
            row.age = Some(state.age);
            row.held = Some(director.is_held());
        }
        if let Some(looks) = &self.looks {
            let state = looks.state();
            row.look = Some(state.to);
            row.mix = Some(state.mix);
            row.look_held = Some(looks.is_held());
        }
        row
    }

    /// The shot the director is on, 0 without a director.
    pub fn shot_index(&self) -> u32 {
        self.director.as_ref().map_or(0, |d| d.state().index)
    }

    /// The look the last step reported, none without looks or before the first step.
    pub fn look_on(&self) -> Option<u32> {
        self.look_on
    }

    /// What the looks tell the scene now (`struct Look`), zeros without looks.
    pub fn look_bytes(&self) -> [u8; super::wgsl::LOOK_SIZE as usize] {
        self.looks
            .as_ref()
            .map_or([0; super::wgsl::LOOK_SIZE as usize], |l| l.state().bytes())
    }

    /// The scene's shots with their sizes as the director lists them (names, sizes and the place
    /// in the burst), empty without a director.
    pub fn shots(&self) -> Vec<(String, ShotSize, u32)> {
        self.director
            .as_ref()
            .map_or_else(Vec::new, Director::shot_list)
    }

    /// The names of the scene's looks, empty without looks.
    pub fn look_names(&self) -> Vec<String> {
        self.looks
            .as_ref()
            .map_or_else(Vec::new, |looks| looks.view().names)
    }

    /// The tier (`quiet`, `normal` or `high`) a section of this level is, by the runtime's
    /// decisions this decider reads.
    pub fn tier_at(&self, level: f32) -> super::Tier {
        let tier = match &self.runtime {
            Some(runtime) => runtime.director.tier_at(level),
            None => RuntimeBindings::embedded().director.tier_at(level),
        };
        match tier.min(2) {
            0 => Tier::Quiet,
            1 => Tier::Normal,
            _ => Tier::High,
        }
    }

    /// What `decision` of a frame at song time `time` is for the event log. `first` is the
    /// scene's first frame, which begins its first shot like a cut.
    pub fn events(&self, time: f64, decision: &Decision, first: bool) -> Vec<LabEvent> {
        if decision.cut.is_none() && decision.look.is_none() && decision.moment.is_none() && !first
        {
            return Vec::new();
        }
        let shots = self.shots();
        let shot_of = |index: u32| shots.get(index as usize);
        let cut_event = |size: ShotSize, fill, strong, hard, dress: Option<Transition>, first| {
            LabEventKind::Cut {
                shot: shot_of(decision.shot).map_or_else(String::new, |s| s.0.clone()),
                size,
                fill,
                strong,
                hard,
                dress: dress.map(|kind| kind.name().to_string()),
                first,
            }
        };
        let mut events = Vec::new();
        match &decision.cut {
            Some(cut) => events.push(cut_event(
                cut.size,
                cut.fill,
                cut.strong,
                cut.motion == ShotMove::Cut,
                cut.dress,
                false,
            )),
            None if first => {
                if let Some((_, size, _)) = shot_of(decision.shot) {
                    events.push(cut_event(*size, 0, false, true, None, true));
                }
            }
            None => {}
        }
        if let Some(look) = decision.look {
            let names = self.look_names();
            events.push(LabEventKind::Look {
                look: names.get(look as usize).cloned().unwrap_or_default(),
            });
        }
        if let Some(moment) = &decision.moment {
            events.push(LabEventKind::Moment {
                strength: moment.strength,
                effect: effect_name(moment.effect),
            });
        }
        events
            .into_iter()
            .map(|kind| LabEvent { time, kind })
            .collect()
    }
}

/// What a moment did, in a word, or none when nothing played.
pub fn effect_name(effect: Effect) -> Option<String> {
    match effect {
        Effect::Dressed(kind) => Some(kind.name().to_string()),
        Effect::Forced => Some("forced".to_string()),
        Effect::Lens(Some(moments::Lens::Flare)) => Some("flare".to_string()),
        Effect::Lens(Some(moments::Lens::Leak)) => Some("leak".to_string()),
        Effect::Lens(Some(moments::Lens::Glitch)) => Some("glitch".to_string()),
        Effect::Lens(Some(moments::Lens::Mosaic)) => Some("mosaic".to_string()),
        Effect::Lens(None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::fixtures;

    #[test]
    fn a_fresh_decider_is_set_like_the_one_it_comes_from_and_has_heard_nothing() {
        let manifest = fixtures::manifest();
        let mut decider = Decider::new(&manifest);
        decider.set_chances(Chances::every(1.0));
        decider
            .director_mut()
            .unwrap()
            .set_pace_override(Some(Pace::Calm));
        decider
            .director_mut()
            .unwrap()
            .set_burst_override(Some(false));
        decider.lens_mut().set_forced(LabLens {
            flare: true,
            ..LabLens::default()
        });
        decider.hold_look(1, &MusicValues::default());
        // Played on a while: what it has heard is not carried over.
        let chart = fixtures::chart(&[(0.0, 20.0, 0.2), (20.0, 40.0, 0.9)]);
        let mut played = decider.clone();
        for frame in 0..600 {
            let t = f64::from(frame) / 60.0;
            let state = chart.at(t);
            let rhythm = crate::chart::trusted_rhythm(Rhythm::default(), Some(&state));
            let music = played.hear(
                &rhythm,
                Some(&state),
                Levels::default(),
                (t, true),
                1.0 / 60.0,
            );
            played.step(1.0 / 60.0, &music, Some(&state), 7, None, 1.0);
        }
        let fresh = played.fresh(&manifest);
        assert_eq!(fresh.settings(), decider.settings());
        assert_eq!(fresh.settings().pace, Some(Pace::Calm));
        assert_eq!(fresh.settings().look, Some(1));
        assert_eq!(fresh.look_on, None);
        assert_eq!(fresh.director().unwrap().state().cuts, 0);
    }
}
