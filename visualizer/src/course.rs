//! The course of the long effects: the lens flare, the light leak, and the glitch and the mosaic
//! over the picture (see "Shared effects" in `docs/music.md`). All four take the same course and
//! follow the music the same way:
//!
//! - It lasts to the end of the phrase it starts in ([`Phrase::beats_left`]), or 4 s without a
//!   followed beat.
//! - It starts from nothing, rises with an eased curve to its peak a third of the way through
//!   ([`shape`]), and thins out to nothing by the end; never a sudden onset.
//! - Its amount at each moment is that shape times the music's `intensity`, smoothed over about a
//!   beat and floored so that it never vanishes mid-way ([`Course::amount`]).
//! - What moves (the glitch's broken rows, the mosaic's blocks) moves on an eased beat time
//!   ([`Course::motion`]): continuous, never a jump, but with a push on every beat, most of a
//!   beat's motion in its first fifth, settling before the next one; and every beat gives what is
//!   there a short accent ([`Course::accent`]) as strong as the beat. Stepping on the beat was
//!   jerky and a smooth clock was limp; this is both continuous and on the
//!   beat. Without a followed beat it pushes on the hits.

use crate::bind::{CourseBinds, MomentBinds, Said};
use crate::scene::MusicValues;
use crate::switching::FOLLOWED;

// The numbers below are the defaults of `course.*` in `runtime.toml`, which the course reads (a
// test pins that they agree); a scene lab can replace them live.

/// Where the course is at its most, as a share of its length.
pub const PEAK: f32 = 1.0 / 3.0;
/// The phrase the length is counted to: a course ends on the next boundary of this many bars,
/// counted from the section's start, and lasts at least [`MIN_BARS`] (so one that starts in a
/// phrase's last bar runs on to the end of the next), at most [`MAX_BARS`].
#[cfg(test)]
pub const PHRASE_BARS: u32 = 4;
#[cfg(test)]
pub const MIN_BARS: u32 = 2;
#[cfg(test)]
pub const MAX_BARS: u32 = 8;
/// Without a followed beat a course lasts this many seconds, and a beat counts as
/// [`UNFOLLOWED_BEAT_SECONDS`] (for the smoothing).
pub const UNFOLLOWED_SECONDS: f32 = 4.0;
pub const UNFOLLOWED_BEAT_SECONDS: f32 = 0.5;
/// The least of the music's part of the amount: at no intensity the effect is at this share of
/// what it is at full intensity, so it never vanishes mid-way.
#[cfg(test)]
pub const LEAST: f32 = 0.35;
/// How sharp the push on each beat is: the ease of the beat time is `1 - e^(-k x)`, scaled to end
/// at 1; at 10, 86% of a beat's motion is in its first fifth, and it moves at most 10 times as
/// fast as the plain beat time.
pub const PUSH_SHARPNESS: f32 = 10.0;
/// The push of a beat with no hit, of the push of a full hit: every beat moves a little.
#[cfg(test)]
pub const PUSH_LEAST: f32 = 0.3;
/// The accent of a push rises over about this much of a beat and settles over the rest of it.
const ACCENT_RISE: f32 = 0.05;
const ACCENT_SETTLE: f32 = 4.0;
/// The most of the accent's curve before it is scaled to peak at 1.
const ACCENT_PEAK: f32 = 0.5332;

/// The shape of a course at `progress` (0 to 1) of its length: 0 at the start, a smoothstep up to
/// 1 at [`PEAK`], and a smoothstep down to 0 at the end; 0 outside.
pub fn shape(progress: f32) -> f32 {
    if !(0.0..1.0).contains(&progress) {
        return 0.0;
    }
    if progress < PEAK {
        smooth(progress / PEAK)
    } else {
        smooth((1.0 - progress) / (1.0 - PEAK))
    }
}

fn smooth(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Where a smoothstep from 0 to 1 reaches `y`.
fn unsmooth(y: f32) -> f32 {
    let y = y.clamp(0.0, 1.0);
    0.5 - ((1.0 - 2.0 * y).asin() / 3.0).sin()
}

/// The ease of a beat, from 0 to 1 as `x` goes from 0 to 1: fast at the start, settling at the
/// end (see [`PUSH_SHARPNESS`]).
pub fn ease(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    (1.0 - (-PUSH_SHARPNESS * x).exp()) / (1.0 - (-PUSH_SHARPNESS).exp())
}

/// The eased beat time of `t` beats: the whole beats as they are, each eased within itself.
/// Continuous and rising; on every beat it pushes on, then settles.
pub fn eased(t: f32) -> f32 {
    let t = t.max(0.0);
    t.floor() + ease(t.fract())
}

/// The accent of a push `x` beats after it, 0 to 1: up within about two frames at 120 bpm (at
/// its most a twelfth of a beat in), settling over the beat, gone at its end.
pub fn accent_shape(x: f32) -> f32 {
    if !(0.0..1.0).contains(&x) {
        return 0.0;
    }
    (1.0 - (-x / ACCENT_RISE).exp()) * (-ACCENT_SETTLE * x).exp() * (1.0 - x) / ACCENT_PEAK
}

/// Whether the beat is followed, so that the tempo can be used for durations.
pub fn followed(music: &MusicValues) -> bool {
    music.locked > FOLLOWED && music.tempo > 0.0
}

/// Where the music is in its phrase: whole bars since the section started (counted as the camera
/// director counts them) and beats since the last bar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Phrase {
    bars: u32,
    beats: u32,
}

impl Phrase {
    /// Take in a frame's music: a section start counts again from 0.
    #[cfg(test)]
    pub fn tick(&mut self, music: &MusicValues) {
        self.tick_with(&crate::bind::RuntimeBindings::embedded().moments, music);
    }

    /// The same, where `moments` say what a section start is.
    pub fn tick_with(&mut self, moments: &MomentBinds, music: &MusicValues) {
        if moments.section_start(music) {
            *self = Self::default();
            return;
        }
        self.bars += music.bars;
        if music.bars > 0 {
            self.beats = 0;
        } else {
            self.beats += music.beats;
        }
    }

    /// Whole bars since the section started.
    #[cfg(test)]
    pub fn bars(&self) -> u32 {
        self.bars
    }

    /// Where in the section it is: whole bars since its start, and beats since the last bar.
    pub fn place(&self) -> (u32, u32) {
        (self.bars, self.beats)
    }

    /// The beats of a course that starts now: to the next [`PHRASE_BARS`] boundary from the
    /// section's start that is at least [`MIN_BARS`] away (counting what of the bar has gone
    /// by), and at most [`MAX_BARS`]. Without a followed beat, [`UNFOLLOWED_SECONDS`] in beats of
    /// [`UNFOLLOWED_BEAT_SECONDS`].
    #[cfg(test)]
    pub fn beats_left(&self, music: &MusicValues) -> f32 {
        self.beats_left_with(&CourseBinds::default(), music)
    }

    /// The same by the `course.length` of `binds`: counted in bars where its source is, else a
    /// time of seconds.
    pub fn beats_left_with(&self, binds: &CourseBinds, music: &MusicValues) -> f32 {
        let seconds = match binds.length(music) {
            Said::Source(_) => {
                let meter = music.meter.max(1.0);
                let into = (self.beats as f32 + music.clock.fract()).min(meter - 0.5);
                let phrase = binds.phrase;
                let mut bars = phrase - self.bars % phrase;
                while (bars as f32) * meter - into < (binds.least_bars as f32) * meter {
                    bars += phrase;
                }
                return (bars as f32 * meter - into).min(binds.most_bars as f32 * meter);
            }
            Said::Value(seconds) => seconds,
            Said::Fallback(..) | Said::Off => UNFOLLOWED_SECONDS,
        };
        seconds / binds.beat_seconds
    }
}

/// A long effect under way (see the module's doc). It is told every frame's music, in
/// [`Course::tick`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Course {
    /// What it reads of the music (`course.*` of `runtime.toml`).
    binds: CourseBinds,
    /// Its length and its age, in beats (of [`UNFOLLOWED_BEAT_SECONDS`] without a followed
    /// beat).
    length: f32,
    age: f32,
    /// The music's intensity, smoothed over about a beat.
    level: f32,
    /// Its beat time, which a restart does not set back: in phase with `clock` while the beat
    /// is followed, else easing to the next whole beat on each hit ([`Course::motion`] eases it).
    beat: f32,
    target: f32,
    /// The strength of the last push, and the beats since it.
    push: f32,
    since_push: f32,
}

impl Course {
    /// A course that starts now, in the music `music` at `phrase`.
    #[cfg(test)]
    pub fn new(phrase: &Phrase, music: &MusicValues) -> Self {
        Self::with(CourseBinds::default(), phrase, music)
    }

    /// The same, reading the music by `binds`.
    pub fn with(binds: CourseBinds, phrase: &Phrase, music: &MusicValues) -> Self {
        let phase = binds.phase(music);
        Self {
            binds,
            length: phrase.beats_left_with(&binds, music),
            age: 0.0,
            level: binds.amount(music),
            beat: phase,
            target: phase,
            push: 0.0,
            since_push: 1.0,
        }
    }

    /// Advance by a frame of `dt` seconds with the music of the frame. While the beat is
    /// followed its beat time runs at the tempo, and a push comes as it passes a whole beat;
    /// without, a hit pushes it on to the next whole beat, which it runs to at a beat of
    /// [`UNFOLLOWED_BEAT_SECONDS`] (faster if hits come faster), so that it eases as a beat does.
    /// A push is as strong as the music's `hit` then, at least [`PUSH_LEAST`].
    pub fn tick(&mut self, dt: f32, music: &MusicValues) {
        let motion = self.binds.motion(music);
        let followed = matches!(motion, Said::Source(_));
        let beat = if followed {
            60.0 / music.tempo
        } else {
            self.binds.beat_seconds
        };
        self.age += dt / beat;
        let follow = 1.0 - (-dt / beat).exp();
        self.level += (self.binds.amount(music) - self.level) * follow;
        self.since_push += dt / beat;
        let push_least = self.binds.push_least;
        let strength = push_least + (1.0 - push_least) * music.hit.clamp(0.0, 1.0);
        if followed {
            let before = self.beat;
            self.beat += dt / beat;
            self.target = self.beat;
            if self.beat.floor() > before.floor() {
                self.push = strength;
                self.since_push = self.beat.fract();
            }
        } else {
            if motion.count() > 0 {
                self.target = self.target.floor() + 1.0;
                self.push = strength;
                self.since_push = 0.0;
            }
            let behind = self.target - self.beat;
            self.beat += behind.min(dt / beat * behind.max(1.0));
        }
    }

    /// How far it is, 0 to 1 (and on past 1 when it is over).
    pub fn progress(&self) -> f32 {
        self.age / self.length.max(1e-3)
    }

    /// Its [`shape`] now.
    pub fn shape(&self) -> f32 {
        shape(self.progress())
    }

    /// How much of it is on now, 0 to 1: its shape times the music's part, from [`LEAST`] at no
    /// intensity to 1 at full.
    pub fn amount(&self) -> f32 {
        self.shape() * (self.binds.least + (1.0 - self.binds.least) * self.level)
    }

    pub fn is_over(&self) -> bool {
        self.age >= self.length
    }

    /// Its eased beat time (see [`eased`]): what moves in it moves by this, on every beat a
    /// push that settles before the next. Continuous, never falling, and on through a restart.
    pub fn motion(&self) -> f32 {
        eased(self.beat)
    }

    /// The beats since it started, linear (the eased [`Course::motion`] dwells on the whole
    /// beats): for what is timed on a cross-rhythm.
    pub fn age_beats(&self) -> f32 {
        self.age
    }

    /// The accent of the last push now, 0 to 1: as strong as the beat was, up at once, settled
    /// by the next beat.
    pub fn accent(&self) -> f32 {
        self.push * accent_shape(self.since_push)
    }

    /// Its length in beats.
    #[cfg(test)]
    pub fn length(&self) -> f32 {
        self.length
    }

    /// This course started again as `fresh` (a course that starts now) would, without a jump: it
    /// takes the new length and goes back on the rising side to where its shape is `share` (the
    /// shape it has now, or less for a stronger start), and rises again from there.
    pub fn restarted(self, fresh: Course, share: f32) -> Self {
        Self {
            length: fresh.length,
            age: unsmooth(share) * PEAK * fresh.length,
            ..self
        }
    }

    /// The course at its peak, as a probe holds it.
    #[cfg(test)]
    pub fn at_peak(self) -> Self {
        Self {
            age: PEAK * self.length,
            ..self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn music(intensity: f32) -> MusicValues {
        MusicValues {
            tempo: 120.0,
            locked: 1.0,
            meter: 4.0,
            intensity,
            ..MusicValues::default()
        }
    }

    /// The shape starts from nothing, peaks a third of the way and is gone at the end.
    #[test]
    fn the_shape_rises_to_a_third_and_falls_away() {
        assert_eq!(shape(0.0), 0.0);
        assert!((shape(PEAK) - 1.0).abs() < 1e-6);
        assert_eq!(shape(1.0), 0.0);
        let curve: Vec<f32> = (0..=300).map(|i| shape(i as f32 / 300.0)).collect();
        assert!(curve[1..100].windows(2).all(|w| w[1] > w[0]));
        assert!(curve[101..300].windows(2).all(|w| w[1] < w[0]));
        // Slowly at first.
        assert!(curve[10] < 0.05, "{}", curve[10]);
    }

    /// A course lasts to the end of the 4-bar phrase it starts in, counted from the section's
    /// start, at least 2 bars, less what of the bar has gone by; 4 s without a followed beat.
    #[test]
    fn a_course_lasts_to_the_end_of_its_phrase() {
        let m = music(1.0);
        let at = |bars: u32, beats: u32| Phrase { bars, beats }.beats_left(&m);
        // At the section's start, a whole phrase of 4 bars.
        assert_eq!(at(0, 0), 16.0);
        assert_eq!(at(1, 0), 12.0);
        assert_eq!(at(2, 0), 8.0);
        // In a phrase's last bar it runs on to the end of the next.
        assert_eq!(at(3, 0), 20.0);
        assert_eq!(at(5, 2), 10.0);
        // Never under 2 bars, counting what of the bar has gone by.
        assert_eq!(at(2, 1), 23.0);
        let all: Vec<f32> = (0..16).map(|b| at(b, 0)).collect();
        assert!(
            all.iter().all(|&beats| (8.0..=32.0).contains(&beats)),
            "{all:?}"
        );
        // The phrase is counted from the section's start.
        let mut phrase = Phrase::default();
        for _ in 0..6 {
            phrase.tick(&MusicValues { bars: 1, ..m });
        }
        assert_eq!(phrase.bars(), 6);
        phrase.tick(&MusicValues { beats: 1, ..m });
        // 7 beats to the boundary are under 2 bars: on to the next.
        assert_eq!(phrase.beats_left(&m), 23.0);
        phrase.tick(&MusicValues {
            cues: 1,
            cue: 1.0,
            bars: 1,
            ..m
        });
        assert_eq!(phrase.beats_left(&m), 16.0);
        // In 3/4, bars of three beats.
        let waltz = MusicValues { meter: 3.0, ..m };
        assert_eq!(Phrase::default().beats_left(&waltz), 12.0);
        // Without a followed beat: 8 beats of half a second.
        let free = MusicValues { locked: 0.0, ..m };
        let mut course = Course::new(&Phrase { bars: 3, beats: 0 }, &free);
        for _ in 0..(3.9 * 60.0) as u32 {
            course.tick(1.0 / 60.0, &free);
        }
        assert!(!course.is_over());
        for _ in 0..12 {
            course.tick(1.0 / 60.0, &free);
        }
        assert!(course.is_over());
    }

    /// The amount is the shape times the music's intensity smoothed over about a beat, floored:
    /// full music, the shape itself; still music, a share of it that never vanishes.
    #[test]
    fn the_amount_follows_the_music() {
        let run = |intensity: f32| {
            let m = music(intensity);
            let mut course = Course::new(&Phrase::default(), &m);
            let mut most = 0.0f32;
            while !course.is_over() {
                course.tick(1.0 / 60.0, &m);
                most = most.max(course.amount());
            }
            most
        };
        assert!((run(1.0) - 1.0).abs() < 0.01);
        assert!((run(0.0) - LEAST).abs() < 0.01);
        assert!((run(0.5) - (LEAST + (1.0 - LEAST) * 0.5)).abs() < 0.01);
        // A change of the music comes in over about a beat (30 frames at 120 bpm), not at once.
        let loud = music(1.0);
        let mut course = Course::new(&Phrase::default(), &music(0.0)).at_peak();
        course.tick(1.0 / 60.0, &loud);
        let after_one = course.amount();
        for _ in 0..29 {
            course.tick(1.0 / 60.0, &loud);
        }
        let after_beat = course.amount();
        assert!(after_one < LEAST + 0.05, "{after_one}");
        assert!(after_beat > 0.7 && after_beat < 0.95, "{after_beat}");
    }

    /// Its eased beat time is continuous and never falls, moves at most as fast as the ease's
    /// peak allows, makes most of a beat's motion early in the beat, and is in phase with the
    /// clock; twice the tempo, twice as many beats.
    #[test]
    fn its_motion_pushes_on_every_beat_and_settles() {
        let m = music(1.0);
        let mut course = Course::new(&Phrase::default(), &m);
        let mut motion = vec![course.motion()];
        // Four beats at 120 bpm, 30 frames each.
        for i in 1..=120 {
            course.tick(
                1.0 / 60.0,
                &MusicValues {
                    clock: i as f32 / 30.0,
                    ..m
                },
            );
            motion.push(course.motion());
        }
        let peak_speed = PUSH_SHARPNESS / (1.0 - (-PUSH_SHARPNESS).exp());
        for w in motion.windows(2) {
            let step = w[1] - w[0];
            assert!(step >= 0.0 && step <= peak_speed / 30.0 + 1e-4, "{step}");
        }
        assert!((motion[120] - 4.0).abs() < 0.01, "{}", motion[120]);
        // Of each beat's motion, most in its first fifth (6 frames).
        for beat in 0..4 {
            let start = beat * 30;
            let early = motion[start + 6] - motion[start];
            let whole = motion[start + 30] - motion[start];
            assert!(early > 0.8 * whole, "{early} {whole}");
        }
        let fast = MusicValues { tempo: 240.0, ..m };
        let mut quick = Course::new(&Phrase::default(), &fast);
        for _ in 0..60 {
            quick.tick(1.0 / 60.0, &fast);
        }
        assert!((quick.motion() - 4.0).abs() < 0.01);
        let again = course.restarted(Course::new(&Phrase::default(), &m), 0.2);
        assert_eq!(again.motion(), course.motion());
    }

    /// Every beat gives an accent as strong as its hit, at least [`PUSH_LEAST`]: up within two
    /// frames, settled by the next beat.
    #[test]
    fn the_accent_follows_the_strength_of_the_beat() {
        let peak_after_a_beat = |hit: f32| {
            let m = MusicValues { hit, ..music(1.0) };
            let mut course = Course::new(&Phrase::default(), &m);
            // Across a beat (frame 30), then the beat after.
            let accents: Vec<f32> = (1..=59)
                .map(|i| {
                    course.tick(
                        1.0 / 60.0,
                        &MusicValues {
                            clock: i as f32 / 30.0,
                            ..m
                        },
                    );
                    course.accent()
                })
                .collect();
            assert!(accents[..28].iter().all(|&a| a == 0.0), "{accents:?}");
            let most = accents.iter().copied().fold(0.0, f32::max);
            let at = accents.iter().position(|&a| a == most).unwrap();
            assert!((29..=32).contains(&at), "{at}");
            assert!(accents[58] < 0.05 * most);
            // Up over about a frame and a half (a punch), down over the beat.
            for w in accents.windows(2) {
                assert!((w[1] - w[0]).abs() <= 0.8, "{w:?}");
            }
            most
        };
        let full = peak_after_a_beat(1.0);
        let none = peak_after_a_beat(0.0);
        let half = peak_after_a_beat(0.5);
        assert!(full > 0.9 && full <= 1.0, "{full}");
        assert!((none / full - PUSH_LEAST).abs() < 0.02, "{none}");
        assert!(none < half && half < full);
    }

    /// Without a followed beat a hit pushes the time on to the next whole beat, easing there as a
    /// beat does, without a jump; without hits it rests.
    #[test]
    fn without_a_beat_the_hits_push_it_on() {
        let free = MusicValues {
            locked: 0.0,
            ..music(1.0)
        };
        let mut course = Course::new(&Phrase::default(), &free);
        let mut motion = vec![course.motion()];
        let mut accents = Vec::new();
        for i in 0..90 {
            let hits = u32::from(i == 10 || i == 50);
            let m = MusicValues {
                hits,
                hit: 1.0,
                ..free
            };
            course.tick(1.0 / 60.0, &m);
            motion.push(course.motion());
            accents.push(course.accent());
        }
        assert!(motion[..11].iter().all(|&m| m == motion[0]));
        assert!((motion[45] - 1.0).abs() < 0.01, "{}", motion[45]);
        assert!((motion[90] - 2.0).abs() < 0.01, "{}", motion[90]);
        let peak_speed = PUSH_SHARPNESS / (1.0 - (-PUSH_SHARPNESS).exp());
        assert!(
            motion
                .windows(2)
                .all(|w| w[1] >= w[0] && w[1] - w[0] <= peak_speed / 30.0 + 1e-4)
        );
        // The hit's accent, up at once, settled by the next half-second beat.
        assert!(accents[..10].iter().all(|&a| a == 0.0));
        assert!(accents[12] > 0.5 && accents[45] < 0.05, "{accents:?}");
    }

    /// Started again, a course goes on from where its shape is, back on the rising side, with
    /// the new length.
    #[test]
    fn a_restart_goes_on_without_a_jump() {
        let m = music(1.0);
        let mut course = Course::new(&Phrase::default(), &m);
        for _ in 0..400 {
            course.tick(1.0 / 60.0, &m);
        }
        let before = course.amount();
        let fresh = Course::new(&Phrase { bars: 1, beats: 0 }, &m);
        let again = course.restarted(fresh, course.shape());
        assert!((again.amount() - before).abs() < 1e-3);
        assert!(again.progress() < PEAK);
        assert_eq!(again.length(), 12.0);
        assert_eq!(again.motion(), course.motion());
    }

    #[test]
    fn the_runtime_file_has_todays_numbers() {
        let c = crate::bind::RuntimeBindings::embedded().course;
        assert_eq!(
            (c.phrase, c.least_bars, c.most_bars),
            (PHRASE_BARS, MIN_BARS, MAX_BARS)
        );
        assert_eq!(c.length.fallback_value(), Some(UNFOLLOWED_SECONDS));
        assert_eq!(c.beat_seconds, UNFOLLOWED_BEAT_SECONDS);
        assert_eq!((c.least, c.push_least), (LEAST, PUSH_LEAST));
        assert_eq!(c.amount.source().name, "intensity");
        assert_eq!(c.motion.source().name, "beats");
        assert_eq!(c.motion.fallback_source().map(|s| s.name), Some("hits"));
        assert_eq!(c.phase.name, "clock");
    }

    #[test]
    fn the_phrase_and_the_floor_follow_the_file() {
        let m = music(1.0);
        let embedded = crate::bind::RuntimeBindings::embedded().course;
        assert_eq!(Phrase::default().beats_left_with(&embedded, &m), 16.0);
        let phrase = crate::bind::tests::embedded_with("phrase = 4", "phrase = 2").course;
        assert_eq!(Phrase::default().beats_left_with(&phrase, &m), 8.0);
        // Without a followed beat, the fallback's seconds.
        let unfollowed = MusicValues { locked: 0.0, ..m };
        let seconds = crate::bind::tests::embedded_with(
            "fallback = { value = 4.0 }",
            "fallback = { value = 6.0 }",
        )
        .course;
        assert_eq!(
            Phrase::default().beats_left_with(&seconds, &unfollowed),
            12.0
        );
        // The least of the amount.
        let peak = |binds| {
            Course::with(binds, &Phrase::default(), &music(0.0))
                .at_peak()
                .amount()
        };
        assert!((peak(embedded) - LEAST).abs() < 1e-4);
        let none =
            crate::bind::tests::embedded_with("when = { least = 0.35 }", "when = { least = 0.0 }")
                .course;
        assert!(peak(none).abs() < 1e-4);
    }

    #[test]
    fn what_pushes_the_motion_without_a_beat_is_the_fallback_of_the_file() {
        let hit = MusicValues {
            locked: 0.0,
            hits: 1,
            hit: 1.0,
            ..music(1.0)
        };
        let moved = |binds| {
            let mut course = Course::with(binds, &Phrase::default(), &music(1.0));
            course.tick(0.1, &hit);
            course.motion()
        };
        let embedded = crate::bind::RuntimeBindings::embedded().course;
        assert!(moved(embedded) > 0.0);
        // Cues push it instead: a hit does not.
        let custom = crate::bind::tests::embedded_with(
            "beat_seconds = 0.5 }\nfallback = \"hits\"",
            "beat_seconds = 0.5 }\nfallback = \"cues\"",
        )
        .course;
        assert_eq!(moved(custom), 0.0);
    }
}
