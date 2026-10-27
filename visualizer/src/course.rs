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
//! - What moves (the glitch's broken rows, the mosaic's blocks) moves continuously in its time in
//!   beats ([`Course::time`]), which runs at the tempo, so it is faster in faster music and nothing
//!   snaps on a beat (not stepped on the beat, but driven by the bpm clock, as the camera is).

use crate::scene::MusicValues;
use crate::switching::{FOLLOWED, SECTION_CUE};

/// Where the course is at its most, as a share of its length.
pub const PEAK: f32 = 1.0 / 3.0;
/// The phrase the length is counted to: a course ends on the next boundary of this many bars,
/// counted from the section's start, and lasts at least [`MIN_BARS`] (so one that starts in a
/// phrase's last bar runs on to the end of the next), at most [`MAX_BARS`].
pub const PHRASE_BARS: u32 = 4;
pub const MIN_BARS: u32 = 2;
pub const MAX_BARS: u32 = 8;
/// Without a followed beat a course lasts this many seconds, and a beat counts as
/// [`UNFOLLOWED_BEAT_SECONDS`] (for the smoothing).
pub const UNFOLLOWED_SECONDS: f32 = 4.0;
pub const UNFOLLOWED_BEAT_SECONDS: f32 = 0.5;
/// The least of the music's part of the amount: at no intensity the effect is at this share of
/// what it is at full intensity, so it never vanishes mid-way.
pub const LEAST: f32 = 0.35;

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

fn followed(music: &MusicValues) -> bool {
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
    pub fn tick(&mut self, music: &MusicValues) {
        if music.cues > 0 && music.cue >= SECTION_CUE {
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
    pub fn beats_left(&self, music: &MusicValues) -> f32 {
        if !followed(music) {
            return UNFOLLOWED_SECONDS / UNFOLLOWED_BEAT_SECONDS;
        }
        let meter = music.meter.max(1.0);
        let into = (self.beats as f32 + music.clock.fract()).min(meter - 0.5);
        let mut bars = PHRASE_BARS - self.bars % PHRASE_BARS;
        while (bars as f32) * meter - into < (MIN_BARS as f32) * meter {
            bars += PHRASE_BARS;
        }
        (bars as f32 * meter - into).min(MAX_BARS as f32 * meter)
    }
}

/// A long effect under way (see the module's doc). It is told every frame's music, in
/// [`Course::tick`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Course {
    /// Its length and its age, in beats (of [`UNFOLLOWED_BEAT_SECONDS`] without a followed
    /// beat).
    length: f32,
    age: f32,
    /// The music's intensity, smoothed over about a beat.
    level: f32,
    /// Beats since it started, which a restart does not set back: what moves in it moves by
    /// this.
    time: f32,
}

impl Course {
    /// A course that starts now, in the music `music` at `phrase`.
    pub fn new(phrase: &Phrase, music: &MusicValues) -> Self {
        Self {
            length: phrase.beats_left(music),
            age: 0.0,
            level: music.intensity.clamp(0.0, 1.0),
            time: 0.0,
        }
    }

    /// Advance by a frame of `dt` seconds with the music of the frame. Its time runs in beats of
    /// the tempo (of [`UNFOLLOWED_BEAT_SECONDS`] without a followed beat): smoothly, unlike
    /// `clock` without a followed beat, which steps on the hits.
    pub fn tick(&mut self, dt: f32, music: &MusicValues) {
        let beat = if followed(music) {
            60.0 / music.tempo
        } else {
            UNFOLLOWED_BEAT_SECONDS
        };
        self.age += dt / beat;
        self.time += dt / beat;
        let follow = 1.0 - (-dt / beat).exp();
        self.level += (music.intensity.clamp(0.0, 1.0) - self.level) * follow;
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
        self.shape() * (LEAST + (1.0 - LEAST) * self.level)
    }

    pub fn is_over(&self) -> bool {
        self.age >= self.length
    }

    /// Beats since it started, which runs on through a restart: the time what moves in it moves
    /// by.
    pub fn time(&self) -> f32 {
        self.time
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

    /// Its time runs smoothly in beats of the tempo: twice the tempo, twice as fast; without a
    /// followed beat in half-seconds, whatever the hits do; a restart does not set it back.
    #[test]
    fn its_time_runs_with_the_tempo_smoothly() {
        let m = music(1.0);
        let mut course = Course::new(&Phrase::default(), &m);
        let mut last = 0.0;
        for _ in 0..60 {
            course.tick(1.0 / 60.0, &m);
            let step = course.time() - last;
            assert!((step - 1.0 / 30.0).abs() < 1e-5, "{step}");
            last = course.time();
        }
        assert!((course.time() - 2.0).abs() < 1e-3);
        let fast = MusicValues { tempo: 240.0, ..m };
        let mut quick = Course::new(&Phrase::default(), &fast);
        for _ in 0..60 {
            quick.tick(1.0 / 60.0, &fast);
        }
        assert!((quick.time() - 4.0).abs() < 1e-3);
        let free = MusicValues {
            locked: 0.0,
            hits: 3,
            ..m
        };
        let mut free_course = Course::new(&Phrase::default(), &free);
        for _ in 0..60 {
            free_course.tick(1.0 / 60.0, &free);
        }
        assert!((free_course.time() - 2.0).abs() < 1e-3);
        let again = course.restarted(Course::new(&Phrase::default(), &m), 0.2);
        assert_eq!(again.time(), course.time());
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
        assert_eq!(again.time(), course.time());
    }
}
