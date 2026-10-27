# What a scene hears

A scene reads the music as named fields of `music` (`struct Music`). This document says what each field means, where its value comes from, what a scene should drive with it, and what a built-in scene must not read. `README.md` describes the frame, the scene format and how the beat tracker and the song's chart work; `src/scene/inputs.rs` fills the raw fields and `src/scene/signals.rs` computes the semantic signals.

The runtime does all the measuring and all the falling back. It decides whether there is a chart, whether the beat can be trusted, and what to tell a scene when something cannot be measured. A scene expresses what it wants to show. It never checks whether a chart exists and never writes a fallback of its own, so every scene gives the same answer to "is there a beat" and "is this a big moment".

Two kinds of field exist:

- The **semantic signals** (`tempo` to `impact`) say what the music is doing. They are the interface for built-in scenes and the one to prefer in any scene.
- The **raw fields** (`song_time` to `hit`) are what the chart and the live analysis found, unprocessed. They stay available to every scene. Built-in scenes may not read the ones that have a semantic replacement (see "Raw fields").

Principles the signals follow:

1. One intent has one signal. "Move at the music's speed" is `tempo`, "now is a time to change the view" is `cues`.
2. A signal is accurate and a scene decides how much to react. How large and how fast a reaction is belongs in the scene's `[[param]]`s (see "Parameters for tuning").
3. The constants below (time constants, thresholds, gaps) are initial defaults. They will be tuned by ear, so a scene must not depend on their exact values.

## Semantic signals

They are added at the end of `struct Music`, after `hit`; the existing fields keep their places. All counts (`cues`, `notes`, `impacts`) are the number of events since the last frame, like `beats`. All moments below (cues, notes, impacts) fire only on a steady frame: the song time moved forward by about a frame's worth and the chart is the same as in the last frame. A seek, a stall, a new track and the chart arriving fire nothing.

"Case A, B, C" mean the three sources of the raw fields (defined under "Raw fields"): A is a chart with a beat, B is a chart without a beat, C is no chart yet, with the live analysis only.

### Tempo and clock

| Field | Type, range | Meaning |
|---|---|---|
| `tempo` | f32, beats per minute, never 0 | The speed to take motion from. |
| `clock` | f32, beats | A beat clock that always runs. |
| `meter` | f32, 2 to 7, never 0 | Beats per bar. |
| `locked` | f32, 0 to 1 | Whether the music's beat is being followed right now. |
| `flow` | f32, beats | A phase that moves on as much as the music does, beat or not. |

**`tempo`.** With a trusted beat it equals `bpm`. Without one it is worked out from `feel`: 3 to 7 Hz map linearly to 70 to 110, and 90 when `feel` has not been measured. When the source changes (the chart arrives or leaves, the beat becomes trusted or is lost) or the target steps by more than 2 bpm in one frame (as where the chart's tempo changes), it glides to the new value with a time constant of 1 s, until it is within 0.5 bpm or for at most 5 s. Otherwise a trusted tempo is told as it comes, and the tempo from `feel` is followed with a time constant of 1 s. Drive with it: speeds of continuous motion (flying, rotating, drifting) as `tempo` times a parameter, and durations given in beats (convert with `60.0 / music.tempo`). Do not read `bpm` or `feel` for this.

**`clock`.** With a trusted beat it is in phase with `beat`. Without one it makes up no tempo: each hit moves it on to the next whole beat, easing there with a time constant of 0.08 s, and it rests between hits, so that what steps on it steps on what is heard (several hits in one frame are several steps). It never goes backwards: where the beat's phase is behind it, the clock waits for it. When the beat becomes trusted, and after a seek or when the chart arrives or leaves (the beat then counts from elsewhere), the integer part is also aligned to the bar by moving forward once, by at most `meter - 1` beats, so that `floor(clock) % meter == 0` marks the downbeat; otherwise only the phase is corrected. The fractional part is the phase within the beat. Drive with it: one step or one pose per beat (`floor` for which beat, `fract` for the phase) and `rv_beat_pulse`. Do not keep your own `last_beat` or your own clock.

**`flow`.** A phase in beats that advances at `tempo` times how loud the three bands are against their average (`band_low`, `band_mid` and `band_high`, about 1 on average, at most 3 counted): it slows in quiet passages, speeds up when the music swells and stops in silence. It is MilkDrop's way of moving with `bass_att`, and it needs no beat. Drive with it: rotation and drift that should breathe with the music. Use `tempo` instead for a steady speed.

**`meter`.** The chart's `beats_per_bar` when there is one, 4 when there is none, limited to 2 to 7. A downbeat is `floor(clock) % u32(meter) == 0`.

**`locked`.** 1 when the beat is trusted and a beat arrived within the last 2.5 beats, else 0, smoothed to the output with a time constant of 0.3 s. It replaces the `sure` that scenes used to compute. Use it to fade grid motion (steps on the beat) in and out, so that a scene is quiet when the beat is lost.

### Energy

| Field | Type, range | Meaning |
|---|---|---|
| `drive` | f32, 0 to 1 | Slow energy, at the scale of a section. |
| `intensity` | f32, 0 to 1 | Fast energy, at the scale of a phrase. |

**`drive`.** With a chart it is the energy of the current section, moving to the next section's energy along a squared curve over the last 4 bars of the section (8 s when there is no beat). Without a chart it is the smoothed loudness with a time constant of 1.5 s. Drive with it: brightness, bloom, colour richness, how busy a scene is. It changes slowly on purpose. It can scale a speed by about plus or minus 20 % but should not replace `tempo`.

**`intensity`.** How loud it is now, relative to the song, with a time constant of 0.8 s. Drive with it: the size of a core, the strength of a light, what changes within a phrase. Do not read `loudness` or `energy` for this.

### Moments

| Field | Type, range | Meaning |
|---|---|---|
| `cues` | u32 | Big moments since the last frame. |
| `cue` | f32, 0 to 1 | Strength envelope of the last cue, decaying with a time constant of 1 s. |
| `cue_in` | f32, seconds, -1 if unknown | Time until the next known cue. |
| `notes` | u32 | Melody notes since the last frame. |
| `note` | f32, 0 to 1 | Strength envelope of the last note. |
| `impacts` | u32 | Hard hits in the middle of a section since the last frame. |
| `impact` | f32, 0 to 1 | Strength envelope of the last impact, time constant 0.35 s. |

**`cues` and `cue`.** A cue is a time that suits a change of view, colour scheme or formation, or a special effect. Where it comes from:

| Case | Cue | Strength |
|---|---|---|
| A or B (a chart) | The start of a section | 1 when the section's energy rises by more than 0.1, else 0.5 |
| Any case, while the beat is followed (`locked` above 0.5) | 8 bars after the last cue of any kind; a section start begins the count again | 0.3 |
| Any case, while the beat is not followed | The first hit 16 s or more after the last cue | 0.2 |

The two fallbacks do not depend on the chart, so that a chart with one long section still has cues.

Two cues are at least 4 s apart. A scene should act on a cue only when its strength is enough for what it does (for example a camera cut at `cue >= 0.5`, a special effect at `cue >= 0.8`). Do not use timers for this, and do not check for a chart.

**`cue_in`.** The time until the next cue that is known in advance. For now that is the chart's `section_left`; it is -1 when nothing is known, which is always the case in C. Use it to start a camera slide early so that the new view arrives on the section's start, and as a lead-in for transitions. Test for `cue_in >= 0.0` before using it.

**`notes` and `note`.** Notes of the melody. With a chart they are onsets of the harmonic stream of strength 0.3 or more. Without one a note is the mid band rising above its recent average (rising threshold 0.35, slow average with a time constant of 0.6 s, with hysteresis, as Badge Tunnel did). Two notes are at least 0.1 s apart. Use them for melodic decoration (a spark, a ripple). Do not write note detection in a scene.

**`impacts` and `impact`.** One hard hit inside a section: the low onset (the chart's `low` stream, or the live kick without a chart) rising above 0.95 while the music is loud enough (`drive > 0.55`), at least 6 s after the last one. An impact does not trigger a camera change; it is for a raised eyebrow, a jump, a single shake. The name avoids `frame.accent`, which is the player's theme colour.

### Bands

| Field | Type, range | Meaning |
|---|---|---|
| `band_low`, `band_mid`, `band_high` | f32, about 1 on average | Levels of the low, mid and high bands, fast up and slow down. |

They rise with a time constant of 0.03 s and fall with 0.2 s. They are three scalars and not a `vec3f` because a `vec3f` in WGSL is aligned to 16 bytes, which would break the sequential layout of `struct Music`. They replace the `bass`, `mids` and `treble` that scenes smoothed themselves. For the shape of the spectrum itself read `rv_spectrum` and `rv_waveform`.

### Fallbacks at a glance

| Signal | A (chart, beat) | B (chart, no beat) | C (live only) |
|---|---|---|---|
| `tempo` | `bpm` | from `feel` | from `feel` |
| `clock` | follows `beat` | steps on hits | steps on hits |
| `flow` | tempo times activity | tempo times activity | tempo times activity |
| `meter` | chart's | chart's | 4 |
| `locked` | 0 or 1 by beats | 0 | 0 |
| `drive` | section energy | section energy | smoothed loudness |
| `cues` | section starts, and every 8 bars without one | section starts, and the first hit after 16 s without one | every 8 bars if locked, else the first hit after 16 s without one |
| `cue_in` | `section_left` | `section_left` | -1 |

## Raw fields

The raw fields keep the order of `MusicValues::fields()` in `src/scene/inputs.rs`. Third-party scenes may read any of them. Built-in scenes may read only the fields marked "yes" in the last column; the others are checked by a test that scans `visualizer/scenes`, and the table says what to read instead.

The three cases:

- **A**: the song has a chart and the chart has a beat. `bpm`, `beat` and `beat_confidence` come from the chart's beats.
- **B**: the song has a chart but the whole song has no steady pulse (a chart without beats). That is the verdict for the song, whatever the live tracker thinks. `bpm`, `beat` and `beat_confidence` read 0, and `beats` and `bars` never fire. Sections and levels still come from the chart.
- **C**: no chart (still being analysed, the analysis failed, or an online track). There is no beat: the live tracker's tempo is often off by half or double, and a beat that does not match what is heard looks worse than none. `bpm`, `beat`, `beat_confidence`, `beats` and `bars` read 0. What the live analysis hears still counts: its onsets (as `hits`), the levels of the spectrum and how fast the music feels (`feel`, so `tempo`). `bpm > 0` therefore always means there is a beat to trust, and only a chart has one.

Without a chart, `music()` fills in as follows: `has_chart` is 0, `beats_per_bar` is 4, `bar` is 0, there is one long section (`section`, `section_label`, `section_repeat`, `section_time`, `section_left`, `section_confidence`, `next_label` and `sections` read 0), `low`, `mid` and `high` are the tracker's kick, snare and hat onsets, `loudness` is the tracker's `energy / 2` limited to 0 to 1, and `section_energy` and `next_energy` both equal that `loudness`. `pulse` is 0, `lead` is -1, and `harmonic`, `phrase`, `lead_onset`, `brightness`, `percussive` and `meter_confidence` read 0.

| Field | Meaning, range | A: chart with beat | B: chart without beat | C: live only | Built-in scene may read | Instead |
|---|---|---|---|---|---|---|
| `song_time` | Seconds into the song | same | same | same | not restricted | |
| `playing` | u32, 1 while the song plays | same | same | same | not restricted | |
| `bpm` | Tempo in beats per minute; 0 means no trusted beat | Chart's, over the 8 beats around now | 0 | 0 | no | `tempo`, `locked` |
| `beat` | Beat count; the integer part is which beat, the fraction is the phase (0 on the beat) | Beats since the song's first beat, so it jumps on a seek | 0 | 0 | no | `clock` |
| `beat_confidence` | 0 to 1, how clearly the music has a beat | Chart's confidence at the nearest beat | 0 | 0 | no | `locked` |
| `beats_per_bar` | Beats in a bar | Chart's meter | Chart's meter | 4 | no | `meter` |
| `bar` | Bar count with the phase in the bar as the fraction | Chart's | 0 | 0 | yes | |
| `meter_confidence` | 0 to 1, how sure the meter is | Chart's | Chart's | 0 | not restricted | |
| `beats` | u32, beats since the last frame | Counted, 0 unless trusted | 0 | Counted while trusted, else 0 | yes | |
| `bars` | u32, bars since the last frame | Counted, 0 unless trusted | 0 | Counted while trusted, else 0 | yes | |
| `sections` | u32, section starts since the last frame | Counted | Counted | 0 | no | `cues` |
| `section` | Index of the section | Chart's | Chart's | 0 | yes | |
| `section_label` | Same value for sections that repeat (every chorus); label A is 0, B is 1 and so on | Chart's | Chart's | 0 | yes | |
| `section_repeat` | How often this label has appeared before | Chart's | Chart's | 0 | yes | |
| `section_energy` | 0 to 1, loudness of this section | Chart's | Chart's | Same as `loudness` | no | `drive` |
| `next_label` | Label of the next section (this one's in the last section) | Chart's | Chart's | 0 | yes | |
| `next_energy` | 0 to 1, energy of the next section | Chart's | Chart's | Same as `loudness` | yes, only to plan ahead with `cue_in` (pick the view or mood the next section arrives with) | for the energy now: `drive` |
| `section_time` | Seconds since this section started | Chart's | Chart's | 0 | not restricted | |
| `section_left` | Seconds until this section ends | Chart's | Chart's | 0 | no | `cue_in` |
| `section_confidence` | 0 to 1, how sure the section boundaries are | Chart's | Chart's | 0 | not restricted | |
| `low` | Onset envelope of the percussive low band: jumps to the onset's strength, then decays | Chart's onsets | Chart's onsets | Tracker's kick (30 to 160 Hz) | yes | |
| `mid` | Same, percussive mid band | Chart's onsets | Chart's onsets | Tracker's snare (160 Hz to 4 kHz) | yes | |
| `high` | Same, percussive high band | Chart's onsets | Chart's onsets | Tracker's hat (5 to 16 kHz) | yes | |
| `harmonic` | Same, pitched sounds | Chart's | Chart's | 0 | no | `notes` |
| `phrase` | Same, phrase starts | Chart's | Chart's | 0 | yes | |
| `lead` | Leading stream: -1 none, 0 low, 1 mid, 2 high, 3 harmonic | Chart's | Chart's | -1 | no | `notes` |
| `lead_onset` | Onset envelope of the leading stream | Chart's | Chart's | 0 | no | `notes` |
| `loudness` | 0 to 1, loudness relative to this song | Chart's curve | Chart's curve | Tracker's `energy / 2`, limited to 0 to 1 | no | `intensity`, `drive` |
| `brightness` | 0 to 1, spectral brightness | Chart's curve | Chart's curve | 0 | yes | |
| `percussive` | 0 to 1, how percussive the sound is | Chart's curve | Chart's curve | 0 | yes | |
| `pulse` | 0 to 1, how clear the rhythm is around now | Chart's curve | Chart's curve | 0 | not restricted | |
| `bass` | Live low level, about 1 on average | Live | Live | Live | no | `band_low` |
| `mids` | Live mid level, about 1 on average | Live | Live | Live | no | `band_mid` |
| `treble` | Live high level, about 1 on average | Live | Live | Live | no | `band_high` |
| `energy` | Loudness of the last second against the last 20 s, about 1 | Live | Live | Live | no | `drive`, `intensity` |
| `has_chart` | u32, 1 when the values come from a chart | 1 | 1 | 0 | no | Scenes do not check this |
| `feel` | How fast it sounds now, in Hz: about 3 for a ballad, 7 for drum and bass; 0 until measured | Chart's curve when the chart has one, else live | same as A | Live | no | `tempo` |
| `song_feel` | `feel` over the whole song | Chart's when it has one, else live | same as A | Live, over the song so far | no | `tempo` |
| `hits` | u32, moments to move on since the last frame | Every trusted beat, strength 1 | Onsets of the leading stream (the harmonic one if none leads) and phrase starts, at least 0.3 | Live kick, snare or hat onsets rising above 0.5 | yes | |
| `hit` | Envelope of the last hit: jumps to its strength (0 to 1), decays with a time constant of 0.2 s | same | same | same | yes | |

Notes on the table:

- Two hits made of onsets are at least 0.25 s apart, and a seek, a stall or the chart arriving fires none. `hits - beats` is the number of hits that were not beats.
- The chart's `bar` is derived from its beats, so in B it is 0.
- `pulse`, `section_time`, `section_confidence`, `meter_confidence`, `song_time` and `playing` have no semantic replacement and are not on the restricted list.
- Fields that no case fills (a 0 in the table) stay 0 for the whole song, so a scene that reads one of them in a case that does not fill it gets a constant.

Always allowed besides these fields: `rv_spectrum`, `rv_waveform`, and every `frame.*` field.

## Recommended practice

### Which signal for which role

| Role in the picture | Use | Do not use |
|---|---|---|
| Speed of continuous motion (advance, rotation, drift) | `tempo` times a parameter for a steady speed (`drive` may scale it by plus or minus 20 %, smoothed with a time constant of 0.5 s or more), or `flow` for motion that breathes with the music | `bpm`, `feel` |
| One pose or step per beat, phase animation | `clock` (`fract` for the phase, `floor` for the beat) with `rv_beat_pulse` | A `last_beat` of your own, your own clock |
| Small movements on the beat (a push, a sway, a turn by one step) | `hits` to trigger, `hit` or an `rv_env` envelope (time constant 0.15 to 0.3 s) | Treating `hits > 0` as a beat |
| Movements at the scale of a bar | `bars`; they only run while the beat is followed, and a scene stays calm when it is not | Inventing bars from onsets |
| Camera moves, colour scheme or formation changes | `cues`, only when the strength is enough. Slide the camera (duration in beats, at least 2 bars, started early with `cue_in` when it is known), or jump and write `post.cut = 1` in the same frame so the transition layer takes over | Timers every N seconds, checks for a chart, transitions of your own |
| How a section looks (every chorus the same) | `section_label` through `rv_label_hash` | |
| Brightness, bloom, colour richness | `drive` (slow) | A flash on every beat |
| Swell of the moment (size of a core, strength of a light) | `intensity` | `loudness`, `energy` |
| One heavy hit inside a section (a jump, a shake) | `impacts`, `impact` | Your own `low > threshold` test |
| Melodic decoration | `notes`, `note` | Your own note detection |
| Spectrum and waveform graphics | `rv_spectrum`, `rv_waveform` | |
| Colour | The hue of `frame.accent` with a `player_colour` parameter | A fixed multicolour rainbow |

### Taste rules

Built-in scenes follow these, and other scenes are asked to.

- **No flashes by default.** `post.flash` is 0 by default. It is used only when `cue >= 0.8` and a parameter turns it on. No pulse of light happens more than twice a second.
- **Calm transitions.** A change of view prefers a slide. A jump is handed to the transition layer with `post.cut = 1`. A hard transition belongs only to the strongest cues and lands on a downbeat.
- **One colour family, night first.** Colours follow the player's accent. The default exposure is dark.
- **Motion on the beat, not light on the beat.** Moving things follow the music; the light stays steady.
- **Quiet when there is no beat.** When the beat is not followed (`locked` near 0), grid motion stops and only continuous motion and the small movements on `hits` remain.
- **Design rules rather than random variety.** Variation comes from rules (what a chorus looks like, what a drop does), not from random picks.

### Patterns

The snippets are sketches; `state` is the scene's `struct State`, and the fields they use are assumed to be declared there.

Speed from `tempo`. The advance is in beats per second of a parameter, smoothed so that a change of source is not seen:

```wgsl
fn update() {
  let dt = rv_dt();
  let goal = music.tempo / 60.0 * params.cruise_per_beat * (0.8 + 0.4 * music.drive);
  if (frame.first == 1u) { state.speed = goal; }
  state.speed = rv_approach(state.speed, goal, dt, 0.5);
  state.z += state.speed * dt;
}
```

One step per beat from `clock`. The step is the integer part, the ease inside the beat is the phase, and the pulse is in beat units:

```wgsl
fn beat_step() -> f32 {
  let whole = floor(music.clock);
  let phase = fract(music.clock);
  let ease = phase * phase * (3.0 - 2.0 * phase);
  return whole + ease;                       // advances one unit per beat
}
fn beat_bounce() -> f32 {
  // A hop on each beat, for motion rather than light; quiet when the beat is lost.
  return params.bounce_amount * rv_beat_pulse(music.clock, 6.0) * music.locked;
}
```

A nudge on hits with `rv_env`. Hits work with or without a beat:

```wgsl
fn update() {
  let dt = rv_dt();
  // music.hit is already an envelope; this one is the scene's own, with a time constant it can tune.
  state.nudge = rv_env(state.nudge, music.hit, dt, params.nudge_time);
  state.pos.y = params.nudge_amount * state.nudge;
}
```

A camera move on cues, started early with `cue_in`. The slide lasts a number of beats, and a scene that does not know the next cue in advance starts when the cue fires:

```wgsl
fn update() {
  let dt = rv_dt();
  let slide_time = params.slide_beats * 60.0 / music.tempo;   // seconds
  let strong = music.cue >= params.cue_min;
  var start = false;
  if (state.sliding == 0u) {
    if (music.cue_in >= 0.0 && music.cue_in < slide_time && state.armed == 1u) {
      start = true;                          // arrive on the section's start
    } else if (music.cues > 0u && strong && state.armed == 1u) {
      start = true;                          // the cue was not known in advance
    }
  }
  if (music.cues > 0u) { state.armed = 1u; }  // the next slide may start
  if (start) {
    state.armed = 0u;
    state.sliding = 1u;
    state.slide_t = 0.0;
    state.from = state.view;
    state.to = rv_hash(frame.seed + u32(state.slide_count)) % 4u;
    state.slide_count += 1u;
  }
  if (state.sliding == 1u) {
    state.slide_t += dt / slide_time;
    if (state.slide_t >= 1.0) { state.sliding = 0u; state.view = state.to; }
  }
}
```

(`armed` is 1 when the scene has not yet moved for the cue it is waiting for. It should start at 1 on `frame.first`. A view picked from `section_label` instead of `rv_hash` makes every chorus use the same view: `u32(rv_label_hash(music.section_label, 7u) * 4.0)`.)

## Bindings

A scene can say what the music drives instead of reading the raw signals and deciding for itself. It declares the values it wants, the **inputs**, and binds each to a **source** with a **how**; the runtime works the value out every frame, falls back where the source cannot be measured, and the shaders read it as `input.<id>`. A scene that declares no `[[input]]` and no `[[bind]]` is not touched by any of this, and the raw fields stay readable as before.

```toml
[[input]]
id = "surge"                 # WGSL: input.surge, always one f32
label = { de = "Schub", en = "Surge" }
range = [0.0, 1.0]           # the values it takes (default [0, 1])
rest = 0.0                   # the value with nothing driving it (default: the bottom of the range)

[[bind]]
target = "surge"             # the input it drives; at most one bind per input
source = "impacts"           # a name from the table below
how = "trigger"              # follow | trigger | step | pick | accumulate
when = { strength = 0.5, release = 0.5 }   # the parameters of the how
fallback = "hits"            # what to do where the source means nothing here
```

`id`s follow the rules of parameter names and are unique among the scene's parameters, buffers, assets and inputs. A mistake (an unknown source, a `when` key the how does not take, a target that is not an input) refuses the scene with a message that names the bind and the item.

### The level

Every frame is at one of three levels, said in one place (`bind::Level::of`): **A** is a chart and a followed beat (`locked` above 0.5), **B** a chart with no beat followed, **C** no chart. They are the A, B and C of "Fallbacks at a glance". A lab can hold the music at a level (key `L` in scene-lab): C passes no chart, B keeps the chart and loses its beat, A is the song as it is. It only takes the music down from what the song has.

### Sources

Each source has a value and, if it is an event, a count of events this frame (the value is then the event's strength). A source **means something** at a level when the music at that level really measures it: its value is always there (the signals fall back as the tables above say), but `bars` never fires without a followed beat and `cue_in` is -1 without a chart.

| Source | Kind | Means something at | |
|---|---|---|---|
| `tempo` `clock` `flow` `meter` `locked` | value | A B C | The tempo and clock signals |
| `drive` `intensity` | value | A B C | The energy signals |
| `hits` `cues` `impacts` `notes` | event | A B C | Count, with the envelope (`hit`, `cue`, ...) as strength |
| `hit` `cue` `impact` `note` | value | A B C | The envelopes alone |
| `bars` `beats` | event | A | Strength 1; they only count while the beat is followed |
| `bar` `beat` | value | A | The absolute bar and beat number, the fraction the phase (the chart's `bar`, and `beat`) |
| `low` `mid` `high` | value | A B C | The percussive onset envelopes (kick, snare, hat): they jump to an onset's strength and decay |
| `cue_in` `section_label` `section_energy` `next_energy` | value | A B | Chart signals |
| `band_low` `band_mid` `band_high` | value | A B C | The smoothed bands |
| `moment.strong` `moment.medium` | event | A B, and A B C | The moments that play effects (`src/moments.rs`): a lift, and another section start or an impact |

### How

| how | Does | `when` (defaults) |
|---|---|---|
| `follow` | The output is the source mapped from a range to a range. | `from = [lo, hi]` (`[0, 1]`), `to = [lo, hi]` (the input's range), `curve` = `linear` \| `smooth` \| `in` \| `out` (`linear`), `smooth` = a time constant in beats (0), `clamp` (true: hold the ends; false: go straight on past them, for an unbounded `clock`). `ease = "out"` eases a beat time first, `"smooth"` with a smoothstep, `ease_power = p` (with `ease = "out"`) with `1 - (1 - x)^p` in place of the engine's own exponential ease. |
| `trigger` | An event starts an envelope; the output goes from the input's rest to the top of its range. Needs an event source. | `strength` = the least strength that counts (0), `attack` and `release` in beats (0 and 0.25: an attack of 0 jumps, the release is a straight fall), `scale` (true: the peak is the event's strength, false: always the top), `decay` = `linear` \| `exp` (`linear`; `exp` takes `release` as a time constant and is cut to 0 under a thousandth), `per` = `beat` \| `second` (`beat`) for `attack` and `release`. Several events in a frame are one. |
| `step` | A counter that goes up by one every `every` beats of the source, on the **eased clock** `floor(t) + ease(fract(t))`: continuous, with a push at each step (the push of `course::eased`). The count is absolute, so a `clock` source steps in phase with the bar. | `every` (1), `wrap` = the count goes round at this (none), `offset` = added to the source before counting, in its units (0), `unit` = `beat` \| `bar` (`beat`: whether `every` counts beats or bars of `meter` beats). `ease = "none"` gives plain `floor` steps; `ease_power` and `ease = "smooth"` as for `follow`. |
| `pick` | An index from 0 to `options - 1`, as a float. A value source (normally `section_label`) picks through the same hash as the transitions (`switching::pick_by_label`), so the same label keeps the same option, and the index changes only when the label does. An event source moves on at each event it counts. | `options` (required, 1 to 64); for an event source also `strength` = the least strength that counts (0), `every` = N: only every N-th counted event moves it (1), `chance` = 0 to 1, a hash of the section and the event's number in it decides, so a song repeats itself (1), `start` = the first index, or `"seed"` to take it from the frame's seed (0), `jump` = `next` \| `random` (`next`; `random` is a hash-picked other index). The events are numbered from 1 in each section. |
| `accumulate` | The output grows by `source * rate` per beat while the beat is followed, per second otherwise; a position from a speed. Starts at the input's rest, is not clamped. | `rate` (1), `per` = `beat` \| `second` (`beat`; `second` for a source that already is a speed in beats, such as `tempo`), `wrap` = goes round at this (none) |

`ease` is only for `follow` and `step`. Effects that move should use an eased clock, not a step per beat: a `follow` of `clock` with `ease = "out"`, or a `step`. Per-beat steps that jump are for what needs them (a pose a beat), and they were jerky on the mosaic and the glitch.

### Composition and parameters

These work with every `how` and close the gaps the Torii Sea pilot found. All are additive: a bind that does not use them means what it did.

```toml
[[bind]]
target = "sway"
source = "tempo"
how = "accumulate"
when = { mul = "drive", per = "second", rate = 0.01, add = { param = "base" } }
```

- **Param references** (pilot gap 1: a scene's parameters could not steer a binding's numbers). Any number of a `when` may be written `{ param = "<id>" }`: a scene `[[param]]` of type `float` or `int`, read every frame from the values the scene has now, so it follows the app's sliders live. A missing or non-numeric parameter refuses the scene at load, naming it. `options`, `ease`, `curve` and the words are not numbers and stay as written.
- **Input sources** (gap 3: a binding could not read another). `source = "input:<id>"` reads the value of an input, which must be declared **earlier** in the manifest than the one the bind drives, so a cycle cannot be written (a later input, itself or an unknown one is refused). The binds run in the order of their inputs, so a reader sees the value of the same frame, whatever the order of the `[[bind]]`s. An input is a value, not an event: a trigger cannot take one. `fallback` stays a registry source.
- **`mul` and `add`** (gap 3). `when.mul = "<source or input:id>"` and `when.add = number` (or a param) change a reading to `reading * mul + add` before the `how` sees it (for an event, its strength; the mul source's own level is not looked at). Tempo scaled by drive for an `accumulate`, or the range of an input driven by another input (a `follow` on the input with `mul = "input:range"`).
- **Trigger envelopes** (gap 2). `decay = "exp"` and `per = "second"`, as in the table above: `when = { release = 0.3, decay = "exp", per = "second" }`.
- **Ease shape** (gap 4). `ease = "smooth"`, or `ease = "out"` with `when = { ease_power = 4 }` (a quartic). Without them, `out` is the engine's exponential ease (sharpness 10).
- **Picks on events** (gap 5). `when = { options = 4, strength = 0.6, every = 2, chance = 0.5, start = "seed", jump = "random" }`. They apply to a pick of an event source (also through an event `fallback`), and a pick by label refuses them.
- **Step phase** (gap 7). `when = { every = 2, unit = "bar", offset = 1 }` steps every two bars, with the clock shifted a beat.
- **Sources** (gap 6). `bar`, `beat`, `low`, `mid` and `high`, in the table above.

### Fallback

Where the source does not mean something at the frame's level, the bind's `fallback` takes over:

| `fallback` | The output is |
|---|---|
| another source's name | that source, read the same way with the same parameters (a trigger needs an event source here too) |
| `{ value = x }` | `x` |
| `"hold"` | the last output |
| `"rest"` | the input's rest |
| left out | the source, at every level (what a scene that reads the raw signal gets) |

The fallback is chosen every frame from the level, so a binding is back on its own source as soon as the level is. A lab reads, for each bind, the level and which means produced the output (`InspectBinding::used`: the source, a fallback source, a value, a hold or the rest).

### Example

```toml
[[input]]
id = "gate"
range = [0, 8]

[[input]]
id = "surge"

[[input]]
id = "light"

[[bind]]                      # a gate that moves with the bar, on the eased clock
target = "gate"
source = "clock"
how = "step"
when = { wrap = 8 }

[[bind]]                      # a lunge on every impact; with nothing as hard, the strong hits
target = "surge"
source = "impacts"
how = "trigger"
when = { release = 0.5 }
fallback = "hits"

[[bind]]                      # one of 3 looks per kind of section; with no chart, round on the cues
target = "light"
source = "section_label"
how = "pick"
when = { options = 3 }
fallback = "cues"
```

In the scene, `input.gate`, `input.surge` and `input.light` are plain `f32`s. A scene lab edits the binds of the scene on screen with `LabCommand::SceneBindings(text)`, TOML holding the `[[input]]` and `[[bind]]` arrays only and checked as the manifest is (a wrong text keeps the old and names the bind in `last_error`): the binds start over, those that did not change keep their state, and only a change of the inputs rebuilds the scene. `Inspect` has, for each bind, its `when` and `fallback` as written (always, not only while it is in use) and the scene's inputs. Editing only the `[[bind]]`s of a scene reloads them without rebuilding the pipelines; editing an `[[input]]` rebuilds the scene, as it changes the layout of `struct Inputs`.

### The runtime's bindings

The runtime's own decisions are bindings too: `runtime.toml` (embedded in the visualizer, read at start) declares them in the shape of a scene's `[[bind]]`, with today's behaviour as its values. The code reads its numbers and its choice of source from the file (`bind::RuntimeBindings`), through the same sources, so a name means one thing in a scene and here. A source that means nothing at the frame's level gives way to the bind's `fallback`; with none the decision is off there. `bars` and `beats` mean something only while the beat is followed. Several binds on one target are an OR. A scene lab replaces the file whole with `LabCommand::RuntimeBindings(text)` (and edits it in place with `toml_edit`: see the scene lab's README); a wrong text is refused with the bind named in `last_error`, and the old bindings stay. `Inspect.runtime_bindings` has a row per bind (the source, the level, which means gave the value).

| Target | Reads | `when` |
|---|---|---|
| `moments.strong` | `cues`; a lift | `strength` 0.8 |
| `moments.medium` | `cues` (a section start, the runtime's one definition of it) and `impacts` | `strength` 0.5, 0 |
| `moments.gap` | `bars`, fallback a bar of 2 s | `every` 8 |
| `director.cadence` | `section_energy`, fallback `drive` | `below` 0.35, `above` 0.65, `every` bars and `seconds` per tier |
| `director.cut` | `bars`, fallback `hits` | `grace` 2 s |
| `director.burst` | `cue_in`, on each `on = beats`, a lift of `to` minus `from` | `lift` 0.1 |
| `switch.place` | `cues` at A and B (a section start that differs), `cues` at C (any) | `strength`, `lift`, `change`, `levels`, `any` |
| `switch.early` | `cue_in`, to start the fade before the place | |
| `switch.timing` | no source | `min` 30, `lead` 8, `relax` 60, `max` 90, `grace` 2 s |
| `switch.forced` | `beats` and `hits`: what ends the wait at `max` | |
| `course.length` | `bars`, fallback 4 s | `phrase` 4, `least` 2, `most` 8 |
| `course.amount` | `intensity` | `least` 0.35 |
| `course.motion` | `beats`, fallback `hits`; starts in phase with `clock` | `push_least` 0.3, `beat_seconds` 0.5 |

The scene sources `moment.strong` and `moment.medium` read the embedded file's moments, not a lab's replacement.

What is not a binding: the shape of the shot sequence and of the burst (four cuts, their sizes), the lab's own forced burst, the effects' curves, the transitions' durations, and the choice of which effect a moment plays (the section's label picks).

## Shots and the director

A scene that has more than one way to look at its subject declares them as shots, and the runtime's camera director decides when the view cuts and to which shot, following the music. The scene frames itself from what the director says and never decides a cut of its own. A scene without `[[shot]]` has no director: it works as it always did, and `shot` reads zeros. The director is per scene, so its counts start again at a scene switch (`src/scene/director.rs`). The numbers below are initial defaults, like the other constants of this document.

### What a scene declares

```toml
[camera]
dutch = 0.35   # the largest dutch angle (roll) in radians; 0 means this scene never tilts
fill = true    # allows the burst of cuts before a section ends (before a lift)
hold = 4       # the shortest shot in bars (default 0): a scene that glides sets it to its glide's length
pace = "normal" # how often the director cuts: sections | calm | normal (default) | busy

[[shot]]
name = "below"     # for people
size = "medium"    # wide | medium | close | detail | top
move = "cut"       # cut | glide
```

- `size` says how near the shot is. `top` is a view from above, a special shot that is never the nearest to another size.
- `move = "cut"` means the view jumps to the new shot (the transition layer may dress the jump later). `move = "glide"` means the scene slides there itself, over its own duration in beats. The director's timing is the same for both; it only reports the move.
- `hold` is the shortest a shot may last, in bars. A scene that glides to a new shot sets it to how long its glide takes, so the shot is reached before the next cut. The cadence below becomes the larger of the section's bars and `hold` (without a followed beat, of the section's seconds and `hold` bars at the tempo, 4 beats to a bar when the meter is unknown). Bursts ignore `hold`: they are hard cuts, so during a burst (`shot.fill > 0`) a scene jumps even to a glide shot.
- `pace` is how often the director cuts this scene, by what suits its subject (each scene declares its own). `normal` is the cadence below; `calm` cuts half as often; `busy` twice as often, with a burst at the end of every phrase of a high section too; `sections` only at a section's start (and in the burst before a lift), and its shots have no planned `length` (-1). `hold` still applies to all of them.
- At least one `[[shot]]` is needed if `[camera]` is given. `fill = true` is only used when the scene has a `close` or a `detail` shot.
- A scene lacking a size gets the nearest one it has.

### What `struct Shot` holds

`shot` is available to `update()` and to every pass, on a binding of its own (`struct Music` is not changed).

| Field | Type, range | Meaning |
|---|---|---|
| `index` | u32 | Which `[[shot]]`, from 0, in the order of `scene.toml`. |
| `cuts` | u32 | How many cuts so far. The scene frames its view again when this changes. |
| `size` | u32 | The size of the shot: 0 wide, 1 medium, 2 close, 3 detail, 4 top. |
| `roll` | f32, radians, signed | The dutch angle for this shot. Fixed within a shot. |
| `seed` | f32, 0 to 1 | Fixed within a shot: which side to come from, a little nearer or farther. |
| `age` | f32, seconds | Seconds since the shot began. |
| `length` | f32, seconds, -1 if unknown | The planned length of the shot (the time to the next cut at the pace in force when it began, a beat in a burst). A section start or a burst may cut it short. -1 before the first cut. |
| `fill` | u32 | 0 normally; 1 to 4 is the position in a burst of cuts. |

The director starts on a wide shot before any cut, with `cuts` 0.

### The rules

**Pace follows the section.** The energy is `section_energy` with a chart and `drive` without. At `pace = "normal"`, a quiet section (below 0.35) cuts every 8 bars, an ordinary one every 4 bars and a high one (above 0.65) every 2 bars (half of the first cadence, so that cuts are not too frequent). A cut lands on a downbeat (`bars > 0`) while the beat is followed (`locked` above 0.5). Without a followed beat the times are 24 s, 12 s and 6 s, and a cut that is due waits for a hit (`hits > 0`), or happens at once 2 s after it was due.

**Size follows the place in the section.** A section start (`cues > 0` with `cue` of 0.5 or more) cuts to a wide shot, which establishes the section. Each later cut steps one size closer: medium, close, detail, and then again from medium. In a high section the steps stop at close, and the cuts alternate between detail, medium and close from there. A quiet section alternates wide and medium. A section start always begins the progression again.

**Consecutive shots differ.** The director never cuts to the shot it is on, unless the scene has only one shot, and then a cut changes `seed`. Among the shots of the size it wants, it picks by a hash of the section's label and the cut's place in the section, so that every chorus picks the same (the hash is `rv_label_hash`'s).

**Dutch angle.** Wide and top shots never tilt. Medium, close and detail shots tilt alternately to the left and to the right, by `dutch * (0.5 + 0.5 * seed)`.

**Glide.** A shot with `move = "glide"` is reported as it is; the scene does the sliding. The director waits no longer and no shorter for it.

**The burst before a change (`fill`).** With the song's analysis, a followed beat, `fill = true` and a near shot, the director cuts on each of the last 4 beats before two places: a section start that lifts the energy (`cue_in` is known and `next_energy` is 0.1 or more above `section_energy`), and, only if `PHRASE_BURSTS` is set (it is off: fewer cuts are wanted), the end of every 8-bar phrase in a high section (phrases count in bars from the section's start). The cuts go to medium, close, detail and top (wide if the scene has no top), with `fill` 1 to 4, tilting alternately. Then the normal rules take over at the section start, or at the phrase start, with a wide shot. A scene with a burst should not also plan a move of its own at those beats.

**The punch on a burst's cuts.** Every cut of a burst (`fill` 1 to 4, no chance roll: bursts are rare) gets a short accent from the transition pass, drawn over the live frame only (it holds no picture, so a scene pays for the pass only while it plays): the view starts scaled up by 7 % and settles to 1 over 0.12 s, eased out; a smear of a few taps along a diagonal that changes side with each cut; a slight colour split outwards from the centre; and a faint flash towards the light tint of the theme colour (at most 15 % of the way, never white). All of it fades with the pop, and its strength rises through the burst, 0.4, 0.6, 0.8 and 1.0 for `fill` 1 to 4, so the last cut hits hardest. A dressed cut or a switch under way takes precedence over a punch. In a scene that draws straight into the output (nothing listed to dress its cuts) the punch starts one frame after the cut, since the cut's own frame is already drawn. A scene lab starts a burst by hand with `B` (the next 4 beats, or 4 cuts a quarter second apart without a followed beat).

A scene that wants to dress its cuts reads `cuts`; the runtime itself knows each cut too (`Scene::cut_this_frame`), which the transition layer will use.

## Parameters for tuning

The standard fixes how the music is measured. How much a scene reacts is the scene's own and is tuned by hand, with the scene reloading as it is edited. For that:

- Every quantity that decides how large or how fast a reaction is becomes a `[[param]]` with a unit and a range. The groups are fixed: beat, picture, camera, colour, performance.
- Names: time constants end in `_time` (seconds) or `_beats` (beats), amounts end in `_amount`, switches are bool.
- A duration that is tied to the music is written in beats and converted with `60 / tempo`, so that a fast song and a slow song feel the same.
- A scene declares no parameter that nothing reads.

## Performance

The runtime keeps the frame rate by adapting. This is how it does so (the details of the controller are in `README.md`):

- **Dynamic resolution.** A factor from 0.5 to 1.0 in steps of 0.05. The internal size is the output size times `[render] scale` times the factor. `[render] scale` is the upper limit. `[[buffer]] scale` is relative to the internal size and shrinks with it.
- **`frame.quality`.** The levels are 1, 0.75, 0.5, 0.25 and 0. The resolution goes down first. Only when it is at 0.5 and the frame is still over budget does the quality go down. Going back up, the quality rises first and then the resolution.
- **Trigger.** The average GPU time of the last 0.5 s over 85 % of the frame's time lowers the level, aiming at 70 %. Only a time under 55 % raises the quality. After a raise that had to be taken back, the next wait doubles.
- **What is timed.** GPU timestamps of the scene and the post-processing. Flutter's own 2 to 4 ms per frame are not part of it.
- The render size is limited to 1920 by 1080, and shaders are compiled on the render thread when a scene is loaded, so switching to a heavy scene can stall a frame.

### Budget

The reference device is an iPhone 13 mini (A15), whose full-resolution output is about 448 by 970.

- **Hard rule.** After optimising, a scene holds 60 fps on the reference device. The runtime may lower the resolution and the quality to get there, but the picture it falls back to must be one that is still good to look at.
- **Goal.** Scene and post-processing take at most 10 ms at full quality and full resolution (quality 1, factor 1), and at most 6 ms at the lowest level (factor 0.5, quality 0), leaving room for Flutter's 2 to 4 ms.
- **The update.** `update()` runs on one GPU thread, and every pass after it waits for it; one thread of a phone GPU is slow. It gets at most 1 ms of the budget on the reference device.

### Measuring

- **Frame rate and frame time:** the benchmark mode, `RUNE_VISUALIZER_BENCH=<seconds per scene>` with `RUNE_VISUALIZER_BENCH_SCENES` to pick scenes (see `README.md`). Its `gpu_avg` is the span from the scene's first pass to the end of post-processing, so on a busy GPU it includes other work (Flutter, the system compositor): it is how long a frame took, not purely what it cost.
- **Cost per pass:** Metal System Trace on the device.
- **That an optimisation keeps the picture:** the ignored test `probe_scene_compare`, which renders a scene on a fixed timeline and seed and compares it pixel by pixel.
- **Measure on the phone.** A desktop GPU is limited by throughput, a phone GPU by the number of loop iterations per pixel and by latency, so timings on a Mac do not stand for the phone.

### Rules for scene authors

1. **Read `frame.quality`, and say in a comment what each level drops**: ray-march steps, samples, layers, particles. Quality may only cut detail, never composition or colour. At level 0 the scene must still be recognisable. A scene that does not read it makes the controller lower the resolution to the minimum and then run out of options.
2. **Count the cost in rays, not pixels.** A long serial loop (ray marching, a pass over every object) does not get faster with a lower resolution. Multiply the step limit by a quality factor, exit early, and replace "every pixel loops over all objects" with tile lists.
3. **Work out sizes in pixels from `frame.resolution` (or the pass's size) each frame.** Do not cache them. A buffer that keeps the last frame is stretched when the size changes, so the effect has to tolerate that.
4. **A heavy scene sets `[render] scale` below 1** (0.75, for example). Buffers for blur or depth of field use 0.5. A deliberately coarse, large-pixel style is a legitimate saving, but it changes the look, so it should be shown to the user before it is chosen.
5. **Bound every loop in `update()`.** It runs serially on the GPU once per frame.
6. **Keep shaders small**, and keep the common files short. Compile time stalls the picture on a scene switch.
7. **Declare `covers = true` only when the scene needs the covers.**

## Shared effects

The transitions and lens effects are the runtime's. A scene says which suit it; the runtime decides when, from the music's moments. All of them read the semantic signals, are off unless listed, and give way to quality.

### What a scene declares

One list, at the top of `scene.toml` (it is a top-level key, so it comes before the tables), for a switch to the scene and for the director's cuts within it alike:

```toml
effects = ["fade", "dissolve", "flash", "glitch", "flare", "leak"]
```

- The names are the runtime's. Transitions: `cut`, `fade`, `dissolve`, `flash` (a colour wash of the theme colour, then overexposure into a light tint of it), `glitch` (rows of broken pieces that shift, smear, streak and flare, in stuttering stages), `mosaic` (classic mosaic: blocks of the picture in flat cells, then the whole frame, its cells growing, then the new picture out of the cells), `cutin` (a torn band with the playing cover, after Persona 5's cut-in). Lens effects: `flare`, `leak`; `blur` is still to come. (`blocks`, a colour-block frame, is now the start of a `flash`; a scene that lists it gets the warning.) A `glitch` or a `mosaic` in the list is a lens effect too: it also plays over the picture, with no cut (to the end of the phrase: thin broken pieces, or small blocks of mosaic, a few, then more, then a few again). A name the runtime does not know is left out with a warning, so a scene written for a newer runtime still loads.
- `flash`, `glitch`, `mosaic` and `cutin` are strong; the rest are light.
- Without the list (also MilkDrop presets): a switch fades or dissolves, a cut is a plain cut, no lens effects. A list that names no plain transition (`cut`, `fade`, `dissolve`) keeps the fade and the dissolve for the plain switches.

### When they play: the moments (`src/moments.rs`)

- **Moments.** A section start that lifts the energy (`cue` 0.8 or more, which the director's burst of cuts leads to) is a strong moment. Another section start (`cue` 0.5 or more) and an impact are medium moments. Nothing else is one: no effect on a plain beat or a plain cut.
- **Chance.** A moment plays an effect by the chance of its strength. The user's setting "effects" has two values: default, 40% of the strong moments and 15% of the medium ones (about two effects in a song); more, 60% and 30%. A scene lab may set any chance (it starts at 100%). The roll is a hash of where the moment is in the song (its section, and which impact of it), so a song plays its effects the same way every time. A medium moment does not play within 8 bars of the last effect; a strong one does.
- **Which.** It depends on what happens at the moment. A scene switch takes a transition of the scene coming (the one going has no say): at a lift whose moment plays, a strong transition the scene lists, picked by the label of the section it lands on; otherwise a lift or a plain change fades and a drop dissolves (a scene that takes neither gets the other, then `cut`). A cut of the director at a moment that plays is dressed with a transition of the list (`fade`, `dissolve`, `flash`, `glitch`, `mosaic`; a cut-in is not drawn over a cut). With no cut, or none that suits, a lens effect of the list plays over the picture (a listed `glitch` or `mosaic` is one of the strong ones). A strong moment takes a strong effect if the scene lists one, a medium moment only a light one, and the section's label picks among them (the same kind of section, the same effect).
- **The burst's punch.** Every cut of the director's burst gets a short punch whatever the list says (see "Shots and the director").

### Transitions (`src/transition.rs`, `transition.wgsl`)

One transition layer serves the switch from one scene to the next and a cut inside a scene. A switch keeps the old picture (or draws the old scene live for a fade or a flash) and combines it with the new scene; a dressed cut keeps the frame before the cut and plays its course over the scene drawing on after it, with half the durations of a switch so that it stays on the beat. Durations are in beats while the beat is followed: a fade 1 beat out and 1 in, a dissolve 2 beats, a flash 1 beat up and 2 down (the new scene goes in at the peak; the first third of the way up washes the picture in the theme colour, after the reference's colour-block frames: still readable, its luma running from a deep shade of the colour through the colour to a near-white tint, and the rest overexposes the washed picture into the flood; the way down is light falling into the new scene), a mosaic 1 beat, a glitch 1 beat, a cut-in a band of ten frames and a beat of hold over the old scene, which plays on, then a tear of ten frames as the new scene goes in (its cracks held for three, then the strips flying apart). A switch that knows its section start (`cue_in`) starts early, so that the new scene shows on it.

The glitch of a switch steps rather than moves: its course is a fixed list of stages in equal shares of its time (`glitch_step`), each held for a few frames: it breaks, breaks harder, stutters back to the first stage, then the new picture comes through its heaviest stage and the breaks die down over it. The mosaic is the classic one: the picture in flat square cells of 3% of the frame's height, each the average of the picture under it (five points of it); no dots, no gaps, no added light, never brighter; blocks of 2 to 4 cells by 1 to 3, one in each slot of 6 by 4 cells, at a place of its own in it, with hard edges. A switch is one continuous course of its progress (`mosaic_switch` in `transition.wgsl`): blocks of the old picture go to cells, more and more; from a quarter of the way the whole frame goes to cells, which grow to twice the size; at half way, under the largest cells, the new picture is behind them; then its cells shrink and fade back to the picture. As lens effects (`Overlay`) they play over the live frame alone, like the punch, and take the course of the long effects (see below, `src/course.rs`): to the end of the phrase, their amount rising and falling with the music, and everything in them moving on the eased beat time, continuous but pushed on every beat (see below). A glitch over the picture: thin rows (a 60th of the height) cut into pieces, each broken for a while of its own (in over half a beat, half a beat to a beat and a half, out over one, then resting 2 to 6), the picture in it shifted by an offset that drifts smoothly and jumps out with each beat's accent, smeared back, half see-through; at its most about 10% of the pieces are broken at once. A mosaic over the picture: each block has a life of its own (in over half a beat, 2 to 4 beats, out over one, then off for 1 to 3), at a phase of its own, so the set changes gradually and is never redrawn; the amount decides, softly, how many of the lives count, so a change of it adds and retires blocks; a block mixes the cells into the picture at most 85%. One at a time: one started while one of the same kind plays lets it go on, one of the other kind takes its place; a switch ends it. The pass runs only while they play.

### Lens effects in the post-processing (`scene/lens.rs`, `post.wgsl`)

With none listed, the post pass does the same work as before (the effects' passes are not encoded and their branches read a uniform of zeros). Each is started at a moment with a strength (1 at a strong moment, 0.6 at a medium one) and runs its course; none is a constant layer. They are drawn in the app's accent (`frame.accent`; a warm orange before the app tells one) and black and white, and none reaches full white.

Both are laid over the tone-mapped picture in display values, blended like a screen, so they lift the dark instead of greying it.

**The course of the long effects** (`src/course.rs`): the flare, the leak and the glitch and mosaic over the picture all take it.

- **Length by the phrase.** An effect lasts to the end of the phrase it starts in: the next 4-bar boundary counted from the section's start (the bars counted as the camera director counts them, a section start counting again from 0), at least 2 bars away counting what of the bar has gone by, at most 8 bars. Without a followed beat, 4 s.
- **Shape.** From nothing, a smoothstep up to the peak a third of the way through, then a smoothstep down to nothing at the end: never a sudden onset.
- **Amount with the music.** At each frame the shape is multiplied by the music's `intensity`, smoothed over about a beat and floored at 0.35: at still music an effect is at 35% of what it is at full, so it never vanishes mid-way; the glitch and the mosaic get more pieces when the music is full and fewer when it holds back.
- **Moving on the beat, eased.** What moves in an effect (the glitch's pieces, the mosaic's blocks) moves on an eased beat time: the whole beats as they are, each eased within itself (`floor(t) + ease(fract(t))`, an ease-out `1 - e^(-10x)` scaled to end at 1), so it is continuous and never jumps, but pushes on at every beat, 86% of a beat's motion in its first fifth, and settles before the next (stepping on the beat was jerky and a smooth clock was limp). Its beat time is in phase with `clock` while the beat is followed and runs at the tempo; without a followed beat each hit pushes it on to the next whole beat, which it runs to as a beat of 0.5 s does. Every beat (or hit) also gives an accent that rises within about two frames at 120 bpm and settles over the beat, as strong as the music's `hit` then, at least 30% of a full one, so strong beats push harder: the mosaic's blocks coarsen their cells by up to half and take a little more of the picture (never brighter), the glitch's broken pieces jump out along their row and drift back.
- **Again.** A flare or a leak started while one runs carries on from the level it has, back on the rising side, with the new phrase's length, so it swells again without a jump; two never add up. A glitch or a mosaic of the same kind goes on as it is.

- **`flare`**: after Keijiro Takahashi's Kino Streak and John Chapman's pseudo lens flare. The highlights above a fixed threshold in the exposed HDR picture (1, with a soft knee from 0.7 to 1.3) are smeared into thin anamorphic streaks by a pyramid that shrinks only across (each level half the width of the one before, a quarter of the internal width and half its height to start, down to a few texels), added back level by level, each wider level weighing less. The streaks are in the theme colour going nearly white where they are bright. Faint ghosts and a halo are sampled from the highlights mirrored through the centre, faded and fringed in colour towards the edges, under a faint fixed lens dirt. Nothing is tracked and nothing adapts to the frame; the highlights are averaged with the frame before's, so one that shimmers about the threshold glows steadily. Its intensity (0.8 at the peak at full intensity of the music) and the reach of its streaks follow its course. It is drawn only from `frame.quality` 0.5 up.
- **`leak`**: a film light leak: a large soft glow (its radius half the frame's long side), its outline torn by low-frequency noise, in the theme colour going warmer towards its source, at about half the light it had at first. Where it comes from is picked at each start, anywhere along any edge, corners included, by a hash of the moment's place in the song (the section, and the bars and beats into it), so the same song does the same thing; within one leak the source keeps its place and travels slowly along the edge (0.15 of an edge over its course), round a corner if it comes to one. Its strength, its size and how far its source reaches in from beyond the edge follow its course.

### In scene-lab

Every effect can be played at once by key or button, and the control panel can make every switch, every cut or every moment use one effect, and set the chance (see `scene-lab/README.md`).

### Lyrics

Lyrics are an overlay drawn by the runtime after tone mapping, so the text is sharp and bloom does not smear it (an optional glow can be added). What a scene can read goes into a struct of its own, `struct Lyrics`, so the layout of `Music` does not change. Its fields are:

- `has_lyrics`,
- the current line's index, the time since it started and the time left in it,
- the progress through the words,
- `lines`: how many lines started in this frame.

A scene may declare in a `[lyrics]` table of `scene.toml` that it shows no lyrics, or give the area and alignment for them and whether their colour follows the theme. It may also read the fields to respond, for example to darken the background a little when a new line starts. The start of a line is not a cue, because lines come too often. Passing the lyrics from Dart to the visualizer is separate work.

### Later effects

Later effects follow the same pattern: they read the semantic signals, are declared in `scene.toml` and off by default, give way to quality, and obey the taste rules.

## Helpers for these patterns

`stdlib.wgsl` provides stateless functions for them.

| Function | Meaning |
|---|---|
| `rv_dt()` | `min(frame.dt, 0.05)`: one upper limit for the time step in every scene |
| `rv_approach(x, goal, dt, tau)` | Exponential approach of `x` to `goal` (not `target`, a reserved word in WGSL) |
| `rv_env(prev, input, dt, tau)` | `max(prev * exp(-dt / tau), input)`, an envelope |
| `rv_beat_pulse(clock, sharpness)` | A spike on the beat, like `rv_pulse` with a beat clock as the phase |
| `rv_label_hash(label, k)` | A number from 0 to 1 that is the same for the same section label and differs for each `k`, so that all scenes hash labels the same way |

The random functions are `rv_unit`, `rv_rand1` and `rv_rand2` (there is no `rv_rand(seed)`).
