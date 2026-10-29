# What a scene hears

A scene reads the music as named fields of `music` (`struct Music`). This document says what each field means, where its value comes from, what a scene should drive with it, and what a built-in scene must not read. [`scenes.md`](scenes.md) is the reference for the scene format itself (the folder, every key of `scene.toml`, and everything a scene's WGSL gets); `README.md` describes the frame and how the beat tracker and the song's chart work; `src/scene/inputs.rs` fills the raw fields and `src/scene/signals.rs` computes the semantic signals.

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

**`locked`.** 1 when the beat is trusted (with a chart: not in a silent stretch, see case A below) and a beat arrived within the last 2.5 beats, else 0, smoothed to the output with a time constant of 0.3 s. It replaces the `sure` that scenes used to compute. Use it to fade grid motion (steps on the beat) in and out, so that a scene is quiet when the beat is lost.

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

Two cues are at least 4 s apart, and with a chart neither fallback (eight bars, or the first hit after a quiet spell) comes within 4 s before a section start the chart knows is coming, so that the gap never drops a section start. A scene should act on a cue only when its strength is enough for what it does (for example a camera cut at `cue >= 0.5`, a special effect at `cue >= 0.8`). Do not use timers for this, and do not check for a chart.

**`cue_in`.** The time until the next cue that is known in advance. For now that is the chart's `section_left`; it is -1 when nothing is known, which is always the case in C. Use it to start a camera slide early so that the new view arrives on the section's start, and as a lead-in for transitions. Test for `cue_in >= 0.0` before using it.

**`notes` and `note`.** Notes of the melody. With a chart they are onsets of the harmonic stream of strength 0.3 or more. Without one a note is the mid band rising above its recent average (rising threshold 0.35, slow average with a time constant of 0.6 s, with hysteresis, as Emoji Tunnel did). Two notes are at least 0.1 s apart. Use them for melodic decoration (a spark, a ripple). Do not write note detection in a scene.

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

- **A**: the song has a chart and the chart has a beat. `bpm`, `beat` and `beat_confidence` come from the chart's beats. Where the song is silent (its loudness under 0.05 of its range for the last second: before the first sound, a gap of a second or more, a silent end or the end of a fade-out) the chart's beats go on at the last tempo, but there is nothing to follow: the frame is told of no beat there, as in B (`bpm` 0, no `beats` or `bars`, `locked` falls), and the beats that went by are not fired when the sound comes back.
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

`id`s follow the rules of parameter names and are unique among the scene's parameters, buffers, assets and inputs: an input's id must differ from every parameter's id (the scene's WGSL reads both, as `params.<id>` and `input.<id>`, and a bind's `{ param = "<id>" }` names a parameter). A mistake (an unknown source, a `when` key the how does not take, a target that is not an input) refuses the scene with a message that names the bind and the item.

### The level

Every frame is at one of three levels, said in one place (`bind::Level::of`): **A** is a chart and a followed beat (`locked` above 0.5), **B** a chart with no beat followed, **C** no chart. They are the A, B and C of "Fallbacks at a glance". A lab can hold the music at a level (key `L` in scene-lab): C passes no chart, B keeps the chart and loses its beat, A is the song as it is. It only takes the music down from what the song has.

### Sources

Each source has a value and, if it is an event, a count of events this frame (the value is then the event's strength). A source **means something** at a level when the music at that level really measures it: its value is always there (the signals fall back as the tables above say), but `bars` never fires without a followed beat and `cue_in` is -1 without a chart.

| Source | Kind | Means something at | |
|---|---|---|---|
| `tempo` `clock` `flow` `meter` `locked` | value | A B C | The tempo and clock signals |
| `drive` `intensity` | value | A B C | The energy signals |
| `section_level` | value | A B C | How high the section is in the song, 0 to 1: half its loudness, half its rank among the song's sections by time (without a chart, the slow energy and its rank among the last minutes). Every song has quiet and high parts; a loud one more high ones. The director's and the looks' tiers read it |
| `audible` | value | A B C | How much is heard: the loudness against a usual one (`loudness / 0.25`, at most 1), so 1 while the music plays, falling with a fade-out and 0 in silence. Multiply a steady motion by it (`mul = "audible"`) so that it slows with the song's own fade and stops where nothing is heard; smooth it in the bind (`smooth`) as the motion needs |
| `hits` `cues` `impacts` `notes` | event | A B C | Count, with the envelope (`hit`, `cue`, ...) as strength |
| `hit` `cue` `impact` `note` | value | A B C | The envelopes alone |
| `bars` `beats` | event | A | Strength 1; they only count while the beat is followed. `bars`, `beats`, `bar` and `beat` mean something only at A (the bindings are gated by the level: a bind with a `fallback` takes it at B and C). A scene that counted `bar % 4` from the raw signals read a bar that stays 0 at C and so got every bar there |
| `cut` | event | A B C | Strength 1; the scene's own camera director cut to another shot in this frame (also one forced from a lab), or the scene's first frame began its first shot. It exists only in a scene's bindings: the runtime's own bindings never see one, and a scene without `[[shot]]`s has no cuts |
| `bar` `beat` | value | A | The absolute bar and beat number, the fraction the phase (the chart's `bar`, and `beat`) |
| `low` `mid` `high` | value | A B C | The percussive onset envelopes (kick, snare, hat): they jump to an onset's strength and decay |
| `cue_in` `section` `section_label` `section_energy` `next_energy` | value | A B | Chart signals. `section` is the chart's index of the section (0, 1, 2, ... down the song: every section has its own number), `section_label` the kind of section (the same number for every chorus) |
| `band_low` `band_mid` `band_high` | value | A B C | The smoothed bands |
| `moment.strong` `moment.medium` | event | A B, and A B C | The moments that play effects (`src/moments.rs`): a lift, and another section start or an impact |

### How

| how | Does | `when` (defaults) |
|---|---|---|
| `follow` | The output is the source mapped from a range to a range. | `from = [lo, hi]` (`[0, 1]`), `to = [lo, hi]` (the input's range), `curve` = `linear` \| `smooth` \| `in` \| `out` (`linear`), `smooth` = a time constant in `per` (0), `per` = `beat` \| `second` (`beat`: the unit of `smooth`), `clamp` (true: hold the ends; false: go straight on past them, for an unbounded `clock`). `ease_power = p` (with `ease = "out"`) eases with `1 - (1 - x)^p` in place of the engine's own exponential ease. The `ease` itself is a key of the `[[bind]]`, beside `how`, not of `when`: `ease = "out"` eases a beat time first, `"smooth"` with a smoothstep. |
| `trigger` | An event starts an envelope; the output goes from the input's rest to the top of its range. Needs an event source. | `strength` = the least strength that counts (0), `attack` and `release` in beats (0 and 0.25: an attack of 0 jumps, the release is a straight fall), `scale` (true: the peak is the event's strength, false: always the top), `decay` = `linear` \| `exp` (`linear`; `exp` takes `release` as a time constant and is cut to 0 under a thousandth), `per` = `beat` \| `second` (`beat`) for `attack` and `release`, `decay_from` = `next` \| `event` (`next`: the fall starts the frame after the event, so the event's own frame shows the peak; `event`: the frame's own time step is already taken off the peak, or with an `attack` already rises by it, as a scene that wrote `k = 1; k *= exp(-dt / tau)` saw it; a `release` of 0 has nothing to fall by and shows the peak that one frame either way). Several events in a frame are one. |
| `step` | A counter that goes up by one every `every` beats of the source, on the **eased clock** `floor(t) + ease(fract(t))`: continuous, with a push at each step (the push of `course::eased`). The count is absolute, so a `clock` source steps in phase with the bar. | `every` (1), `wrap` = the count goes round at this (none), `offset` = added to the source before counting, in its units (0), `unit` = `beat` \| `bar` (`beat`: whether `every` counts beats or bars of `meter` beats). `ease = "none"` gives plain `floor` steps; `ease_power` and `ease = "smooth"` as for `follow`. |
| `pick` | An index from 0 to `options - 1`, as a float. A value source (normally `section_label`) picks through the same hash as the transitions (`switching::pick_by_label`), so the same label keeps the same option, and the index changes only when the label does. An event source moves on at each event it counts. | `options` (required, 1 to 64); for an event source also `strength` = the least strength that counts (0), `max_strength` = the most (none: with `strength`, a band of strengths), `every` = N: only every N-th counted event moves it (1), `chance` = 0 to 1, a hash of the section and the event's number in it decides, so a song repeats itself (1), `start` = the first index, or `"seed"` to take it from the frame's seed (0), `jump` = `next` \| `random` (`next`; `random` is a hash-picked other index). The events are numbered from 1 in each section. A pick by label also takes `start = "seed"`: the frame seed of the bind's first frame salts the label's hash, so one play of a song gets one assignment (the same label always the same option) and another play may get another. |
| `accumulate` | The output grows by `source * rate` per beat while the beat is followed, per second otherwise; a position from a speed. With `per = "event"` it counts events instead: it grows by `rate` for each event, with no time in it. Starts at the input's rest, is not clamped. | `rate` (1), `per` = `beat` \| `second` \| `event` (`beat`; `second` for a source that already is a speed in beats, such as `tempo`; `event` needs an event source, also as the fallback), `strength` = for `event`, the least strength of a frame's events that counts (0), `reset` = an event source that sets the sum back to 0 on its event, after this frame is added, so the sum is 0 on the event (none), `wrap` = goes round at this (none) |

`ease` is only for `follow` and `step`, and is written on the bind (`ease = "out"` beside `how`), not inside `when`. Effects that move should use an eased clock, not a step per beat: a `follow` of `clock` with `ease = "out"`, or a `step`. Per-beat steps that jump are for what needs them (a pose a beat), and they looked jerky on the mosaic and the glitch.

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
- **Ease shape** (gap 4). On the bind, `ease = "smooth"`, or `ease = "out"` with `when = { ease_power = 4 }` (a quartic; only `ease_power` is in `when`). Without them, `out` is the engine's exponential ease (sharpness 10).
- **Picks on events** (gap 5). `when = { options = 4, strength = 0.6, every = 2, chance = 0.5, start = "seed", jump = "random" }`. They apply to a pick of an event source (also through an event `fallback`), and a pick by label refuses them.
- **Step phase** (gap 7). `when = { every = 2, unit = "bar", offset = 1 }` steps every two bars, with the clock shifted a beat.
- **Sources** (gap 6). `bar`, `beat`, `low`, `mid` and `high`, in the table above.

The scene migrations then found ten more gaps, closed in the same way (all additive):

- **`section` source.** The absolute section index, where the chart is (A and B), as `section_label` is: `source = "section"`.
- **Trigger decay on the event's frame.** `when = { release = 0.3, decay = "exp", decay_from = "event" }`: the envelope is set to its peak and decayed by the frame's step in that same frame (see the table: also for a linear fall and an `attack`).
- **An event counter that does not wrap.** `source = "beats", how = "accumulate", when = { per = "event", rate = 1 }` adds `rate` for each event of the frame (those that pass `strength` if given), with no time involved and no wrap unless `wrap` says so: beyond a pick's 64 options, and not late at level C as a clock-driven `step` is.
- **Accumulate reset, and the `cut` source.** `when = { per = "second", reset = "cut" }` sets the sum back to 0 on the scene's own camera cut (after this frame adds: the sum is 0 on the cut), so a drift starts over with each shot. Any event source can reset. A scene hears of a cut in the frame it is made (the director decides before the bindings run); a cut a lab forces counts too.
- **Numbers as small expressions.** Wherever a `when` number may be `{ param = "id" }`, `{ param = "id", mul = k, add = c }` is the parameter times `k` plus `c`: `when = { rate = { param = "flow", mul = 0.06 } }`. Also in the pairs: `to = [0, { param = "span", mul = 2, add = 1 }]`. `when.mul` may be a number (or such an expression) as well as a source name: `when = { mul = 0.5, add = 0.1 }`.
- **Follow smoothing in seconds.** `when = { smooth = 0.4, per = "second" }`; the default is beats.
- **Pick bands and seeds.** `when = { options = 4, strength = 0.2, max_strength = 0.6 }` counts only events of that band. `source = "section_label", how = "pick", when = { options = 5, start = "seed" }` gives each kind of section a random option that is stable within a play. With `start = "seed"` and `jump = "random"` on an event pick, a scene that wanted a first option and every later one drawn from the play's seed has both; what stays in the scene's WGSL is its own `random(...)` draws, which are not a pick.

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
| `director.cadence` | `section_energy`, fallback `drive` | `below` 0.35, `above` 0.65, `seconds` per tier (24, 16, 10), `gap` 6 s, `lead` 4 s |
| `director.sizes` | no source | `quiet`, `normal`, `high`: five numbers each, how much wide, medium, close, detail and top are wanted in a section of that tier (3, 3, 1, 0, 1 / 2, 3, 2, 1, 1 / 1, 2, 3, 2, 1) |
| `director.cut` | `bars`, fallback `hits` | `grace` 2 s |
| `director.burst` | `cue_in`, on each `on = beats`, a lift of `to` minus `from` | `lift` 0.1, `chance` 1 (0.5 in `runtime.toml`) |
| `look.change` | `cues` (a section start with a chart; any cue without) | `strength` 0.5, `gap` 8 s, `strength_unlabelled` 0, `gap_unlabelled` 30 s, `fade` 1 beat, `fade_seconds` 0.5 s |
| `course.length` | `bars`, fallback 4 s | `phrase` 4, `least` 2, `most` 8 |
| `course.amount` | `intensity` | `least` 0.35 |
| `course.motion` | `beats`, fallback `hits`; starts in phase with `clock` | `push_least` 0.3, `beat_seconds` 0.5 |

The scene sources `moment.strong` and `moment.medium` read the embedded file's moments, not a lab's replacement.

What is not a binding: the shape of the shot sequence and of the burst (a cut on each of the last beats before a section, one for each shot the scene declares), the lab's own forced burst, the effects' curves, the transitions' durations, and the choice of which effect a moment plays (the section's label picks).

## Shots and the director

A scene that has more than one way to look at its subject declares them as shots, and the runtime's camera director decides when the view cuts and to which shot, following the music. The scene frames itself from what the director says and never decides a cut of its own. A scene without `[[shot]]` has no director: it works as it always did, and `shot` reads zeros. The director is per scene, so its counts start again at a scene switch (`src/scene/director.rs`). The numbers below are initial defaults, like the other constants of this document.

### What a scene declares

```toml
[camera]
dutch = 0.35   # the largest dutch angle (roll) in radians; 0 means this scene never tilts
dutch_in = "bursts"  # optional, default "all": tilt only a burst's cuts (a tilt on every cut was too much for Neon Street)
hold = 8.0     # the shortest shot in seconds (default 0): a scene that glides sets it to its glide's length
pace = "normal" # how often the director cuts: sections | calm | normal (default) | busy

[[shot]]
name = "below"     # for people
size = "medium"    # wide | medium | close | detail | top
move = "cut"       # cut | glide
weight = 1.0       # optional, default 1: how often it is picked against the scene's other shots

# A shot of a size other than wide may still open a section.
[[shot]]
name = "from above"
size = "top"
opens = true       # optional: may open a section (default true for a wide shot, false otherwise)
at_least_once = true  # optional, default false: the song is promised a cut to this shot (see "A shot the song is promised")

# Optional, one to four, in the order the burst cuts to them. Read the warning below first.
[[burst]]
name = "in"        # for people
size = "close"     # as for a shot; its move is always a hard cut
```

- `size` says how near the shot is. `top` is a view from above, a special shot: a different size from every other, and it opens a section only if it says `opens = true`.
- `opens` says whether the shot may open a section. Left out, a wide shot may and the others may not. A scene whose wide shot is not its best opening, or that wants a view from above to establish a section, says so here.
- `tiers` says which kinds of section, by energy, the shot suits: any of `"quiet"`, `"normal"`, `"high"` (the section's tier, see `director.cadence`), left out for all. The director picks only among the shots that suit the section, and among all if none does (Neon Street's far views are not for its climax).
- `dutch` says whether the shot tilts by the dutch angle whatever `[camera] dutch_in` says (optional): `true` tilts it even in a scene that tilts only its bursts, `false` keeps it level. Left out, the shot does as `dutch_in` says. A wide or top shot never tilts, so it may not say `true` (Neon Street's street side tilts).
- `at_least_once` promises the shot to a song that has a chart: it is picked by chance like any other (its `weight`, `tiers` and the size wishes), and if it has not been on by the last section that suits it, the director cuts to it there. Read "A shot the song is promised" below. The key is not `once`, which would read as "only once": the shot may come as often as chance picks it, and the promise is only a floor (Paperclip's circling shot, which only high sections suit, could be left out of a whole song by the weights).
- `longest` is the longest the shot may last, in seconds (optional): after it the director cuts away at the next place a cut lands on, however long the cadence's wait, though never within the gap. For a shot that tires the eye if it stays, as a close one beside moving things can.
- `weight` is how often the director picks the shot against the scene's others (a number, 0 or more, default 1): a shot of 2 is picked twice as often as one of 1, and one of 0 is never picked while another has a chance. A scene whose shots all have weight 0 is refused. `[[burst]]` takes neither `opens` nor `weight`.
- `move = "cut"` means the view jumps to the new shot (the transition layer may dress the jump later). `move = "glide"` means the scene slides there itself, over its own duration in beats, and a smooth move is not dressed: the moment that would have dressed it with a transition plays a light lens effect of `lens` instead (a flare, a leak; never a glitch or a mosaic over it, which `transitions` keeps for the hard cuts). The exception is a hard cut: a burst's cuts and a strong cut (the cut a burst leads to, the start of a high section) jump even to a glide shot, with their strong transition, and the scene is told so by `shot.hard`. The director's timing is the same for both moves.
- `hold` is the shortest a shot may last, in seconds. A scene that glides to a new shot sets it to how long its glide takes, so the shot is reached before the next cut. It only ever raises the gap below (`director.cadence`'s `gap`) for this scene: no cut, of the cadence or of a section start, comes sooner after the last than the larger of the two. Bursts ignore it: they are hard cuts, so a scene jumps even to a glide shot there (`shot.hard` is 1, as it is on the cut a burst leads to).
- `pace` is how often the director cuts this scene, by what suits its subject: each scene declares its own. It scales the seconds of the cadence below: `normal` as they are, `calm` 1.5 times as long, `busy` 1.5 times shorter (and then a burst at the end of every phrase of a high section too, if the scene declares a burst); `sections` has no cadence, only a cut at a section's start (and in the burst before a lift), and its shots have no planned `length` (-1). `hold` still applies to all of them.
- At least one `[[shot]]` is needed if `[camera]` or `[[burst]]` is given. A scene may lack any size: the director picks among the shots it has.
- `[camera] fill` is gone: a scene that still has it is refused, with a message that says to declare `[[burst]]` shots instead.
- `[[burst]]` declares the shots of the burst of cuts before a section that lifts the energy. They are the burst's own: ordinary cuts and section starts never pick them, and the burst never uses a `[[shot]]`. Their `index` in `struct Shot` is the number of `[[shot]]`s plus their place among the bursts, from 0. A scene with none has no burst, ever (nor does the scene lab's `B` do anything).

> **Warning to authors: a burst is the hardest cut to get right.** It is four hard cuts in two seconds, then the section start. Declare one only with shots made for it: each nearer than the one before, or from a new angle, a sequence that builds to the section instead of four unrelated views. Check it beat by beat in the scene lab (`B` forces one, and a lift in the song shows the real one) before it ships, with your own eyes at the song's tempo, not at a still frame. And leave it out when you are unsure: a scene with no `[[burst]]` cuts well without one, and a bad burst is the first thing a viewer sees. Each of the seven built-in scenes declares four, made for it.

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
| `hard` | u32, 0 or 1 | 1 when this shot was entered by a hard cut and the scene is to jump to it: a cut to a `move = "cut"` shot, any cut of a burst, and a strong cut (the cut a burst leads to, the start of a high section), even to a `move = "glide"` shot. 0 when the scene is to glide to it. A gliding scene jumps when `hard` is 1 and slides otherwise. 1 for the scene's first shot, which has nothing to glide from. |

The director starts on a shot that opens a section (or the nearest to wide) before any cut, with `cuts` 0 and `hard` 1. The struct is 48 bytes: `hard` and three words of padding after the eight words above.

### The rules

**Pace follows the section, in seconds.** The section's tier comes from its level in the song, `section_level` (half its loudness, half its rank among the song's sections: with a chart the section's energy and its rank by time among all the sections; without, the slow energy and its rank among what the song has had in the last minutes), so that every song has quiet and high parts and a song loud throughout more high ones (by loudness alone, most of a loud song was "high"). The same tiers pick the looks and the shots that suit a section. At `pace = "normal"`, a quiet section (below 0.35) cuts every 24 s, an ordinary one every 16 s and a high one (above 0.71: the first chorus of a pop song mastered loud throughout came out at 0.73) every 10 s, at any tempo and with a followed beat or without (counted in bars, a fast song cut every few seconds, far too often). `calm` multiplies the seconds by 1.5 and `busy` divides them by 1.5. A cut that is due lands on a downbeat (`bars > 0`) while the beat is followed (`locked` above 0.5); without a followed beat it waits for a hit (`hits > 0`), or happens at once 2 s after it was due.

**The cuts make way for each other.** No cut that is not inside a burst comes less than `gap` seconds (6, or the scene's `hold` if that is longer) after the last one. A cadence cut due sooner waits for the gap; a section start within the gap does not cut (the shot goes on, but the section's progression still starts again, so its next cut opens the section). With the next section start known ahead (`cue_in` of 0 or more), a cadence cut due less than `lead` seconds (4) before it waits, or less than `gap` if that is longer (a cut then would hold the section start back), and the section start cuts instead. Without a known section ahead (no chart) only the gap applies. The start of the first section of a scene, before the scene has cut at all, is not held by the gap.

**A section start opens with a shot that opens.** A section start (`cues > 0` with `cue` of 0.5 or more) cuts to one of the scene's shots that `opens` (a wide one unless the scene says so), picked by `weight` from the section's label alone, so that the same kind of section opens the same way whatever came before it. Only where that would be the shot on screen (a cut to it would only jump), or after a burst a shot of the burst's last size (after a burst that ends from above, another view from above would only seem to move in), it picks again away from the shot on, unless that is the scene's only shot: with no other shot that opens, or none at all, it takes the one nearest to wide. A section start always begins the section again: a start the gap held back leaves the next cut to open the section.

**The other cuts are picked at random by weight, within a few rules.** A cut within a section goes to another size than the section's last shot (`top` differs from every size), so the view never repeats itself: the section's own last shot, not whatever is on screen, so that its shots follow from its label alone; a scene whose shots are all of one size cuts to any other shot, and a scene with one shot stays on it and only changes `seed`. Each shot that may be picked has a chance of its `weight` times what `director.sizes` wants of its size in the section's tier (quiet, normal or high, the same energy that sets the cadence): a quiet section wants wide and medium, a high one close and detail, and a shot of a size the tier gives 0 is not picked. If every chance is 0 the weights alone decide, and if those are 0 too, evenly. The choice is a deterministic roll of a hash of the section's label and the cut's number in the section (`rv_label_hash`'s), so a song picks the same shots each time it plays and every chorus the same (only its opening shot may differ, where the section before ended on it). A fixed order of sizes is mechanical and predictable, so only these rules are fixed, not the order.

**Dutch angle.** Wide and top shots never tilt. Medium, close and detail shots tilt alternately to the left and to the right, by `dutch * (0.5 + 0.5 * seed)`. The side a section starts leaning to and the `seed` of each shot come from the section's label and the cut's place in it, so a chorus leans the same way each time.

**Glide.** A shot with `move = "glide"` is reported as it is; the scene does the sliding. The director waits no longer and no shorter for it. A glide is not dressed: at a moment that plays an effect, a cut with `Cut::motion` of `Glide` gets no transition (no fade, dissolve, flash, glitch or mosaic over it) and the moment plays a light lens effect of the scene's `lens` (`moments::lens_over_glide`: a flare or a leak, none if `lens` has neither). A strong cut stays a hard cut: the director reports it with `motion` `Cut` and `shot.hard` 1 whatever the shot's `move`, and the scene must jump, so a gliding scene reads `shot.hard` (not `shot.fill`, which is 0 on the cut a burst leads to) to know whether to slide or jump. In scene-lab, the cut override still dresses every cut, glides too: it is a test tool.

**The burst before a change (`fill`).** With the song's analysis, a followed beat and the scene's `[[burst]]` shots, the director cuts on each of the last beats before two places, one beat for each declared shot (1 to 4): a section start that lifts the energy (`cue_in` is known and `next_energy` is 0.1 or more above `section_energy`), and, only if `PHRASE_BURSTS` is set (it is off, for fewer cuts) or the scene's pace is busy, the end of every 8-bar phrase in a high section (phrases count in bars from the section's start). The cuts go to the declared shots in the order of the file, with `fill` 1 to N, tilting alternately as any shot of their size does (wide and top never tilt), and are always hard cuts. The cut the burst leads to (the section's or the phrase's start, or the opening a lab's burst ends on, a beat after its last) is a hard cut too, even to a `move = "glide"` shot (`shot.hard` 1), and always lands on a strong transition of the scene's list (picked by the section's label, as a strong moment's is), whatever the roll of the moment: four hard cuts ending on a plain one stopped short. Then the normal rules take over at the section start, or at the phrase start, with a shot that opens. A scene with a burst should not also plan a move of its own at those beats.

**A burst only sometimes, a climax always on a cut.** Not every lifting section start gets its burst: `director.burst`'s `chance` (0 to 1; 1 where the file leaves it out, so older files behave as before; `runtime.toml` sets 0.5) is the chance of it, rolled for the coming section from its number in the song and its label (the same hash as a moment's roll, with its own salt), so a song bursts before the same sections on every play and the sections differ, since a burst with the switch looks good, but it need not be every time. A burst not taken holds nothing back: the cadence waits for the burst's room only before a burst that will be taken. A scene that declares a burst bursts at least once in a song, and the guarantee is not the first lift but the last: if none of the earlier ones fired, the last one does. Every lifting section start rolls by the chance and nothing is forced early. At the song's last lift, if no burst has begun in this song, the burst is taken whatever the roll. The last lift is the one after which no section boundary of the chart rises by `lift` (0.1) or more: `ChartState::later_rise` gives the biggest rise among the boundaries after the coming section start, from the chart's section energies, and the scene hands it to the director each frame (`Director::set_song_ahead`; none without a chart, where there is no burst). It reads the energies of consecutive sections, as the default `from` and `to` of `director.burst` do. The promise is lost where the burst cannot fit (the last cut is within the gap of its first), and a scene that comes in after the song's last lift has none. The song time going back (a new song, or a seek) starts the song's account again. The phrase bursts of a busy scene are left as they were, and the lab's own (`B`) always bursts. Whatever the roll, the start of a section in the high tier (a section start only exists with a chart) is always a cut, even within the gap, and it lands on a strong transition of the scene's list, as the cut after a burst does (`Cut::strong`, `CutInfo::strong`): a climax is always entered on a cut. It is a hard cut also in a gliding scene, which jumps there (`shot.hard`). A section start into a quiet or normal tier is as before: no cut within the gap, and a dressing only by the moment's chance. In scene-lab, "Bursts before a lift" (auto, always, never; `LabCommand::BurstsWith`) replaces the chance and the last lift's promise for every scene, and the director's readout shows the chance and whether the coming section will burst.

**A shot the song is promised (`at_least_once`).** Like the burst's promise, it is random first and a floor after. The director remembers which `[[shot]]`s have been on in this song (cut to by any rule, a lab's hold too) and forgets them when the song time goes back (a new song, or a seek) or the chart's list of sections is replaced. For a shot that says the key and has not been on, the song's last chance is the last section that suits it by its `tiers` (the section's level, the same `section_level` that sets its tier, against `director.cadence`'s `below` and `above`); if the song has no section that suits it, the song's highest section (the highest level, the last of equals). In that section the director cuts to the shot at the first cut after the section's opening: the opening keeps its own rule (a section opens on a shot that opens, picked by label, and if the promised shot itself opens it may be that opening, which counts), and the cut goes to the promised shot in place of the pick, whatever sizes the section has had. It also makes sure the cut comes: if the cadence's next cut would not come in time before the section ends (it would fall within the room `waits_for_section` keeps clear before the next section start, or the scene's pace cuts only at section starts), the director cuts at the gap (`gap`, or the scene's `hold`) from the opening, landing on the same event as any cadence cut, and that cut ignores the room before the next section start. A section is a last chance only if it is long enough for that: the gap, plus the larger of `lead` and the gap, plus the cut's `grace` (14 s by default). If the last section that suits a shot is too short, the one before it that suits is the last chance; if none that suits is long enough, the song's highest that is. A song with no section that long promises nothing. The director learns the sections ahead from the chart: `ChartState::song` lists every section with its start, end and level (made once with the chart and shared, so a frame only compares the list's identity), and the scene hands it to `Director::set_song`. **Without a chart nothing is promised**: there are no sections ahead, so a shot of the key is only picked by chance. A scene joining a song after the last chance has passed, or a seek past it, loses the promise until the song time goes back. Scene-lab's Inspect shows `owed this song` beside a shot the song still owes.

The burst and the section start right after it are one group, which the gap does not hold apart, and the cadence counts on from that section start. The group is kept clear on both sides: no cadence cut comes within the gap before the burst's first cut (with the lift known ahead, the cadence waits), and a burst whose first cut would come within the gap of the last cut is left out whole, since it would not be a burst but a pile of cuts.

**The punch on a burst's cuts.** Every cut of a burst (`fill` 1 to 4, no chance roll: bursts are rare) gets a short accent from the transition pass, drawn over the live frame only (it holds no picture, so a scene pays for the pass only while it plays): the view starts scaled up by 7 % and settles to 1 over 0.12 s, eased out; a smear of a few taps along a diagonal that changes side with each cut; a slight colour split outwards from the centre; and a faint flash towards the light tint of the theme colour (at most 15 % of the way, never white). All of it fades with the pop, and its strength rises through the burst, 0.4, 0.6, 0.8 and 1.0 for `fill` 1 to 4, so the last cut hits hardest. A dressed cut or a switch under way takes precedence over a punch. In a scene that draws straight into the output (nothing listed to dress its cuts) the punch starts one frame after the cut, since the cut's own frame is already drawn. A scene lab starts a burst by hand with `B` (the next beats, one per declared shot, or the same cuts a quarter second apart without a followed beat; nothing for a scene that declares none).

A scene that wants to dress its cuts reads `cuts`; the runtime itself knows each cut too (`Scene::cut_this_frame`), which the transition layer will use.

## Looks

A scene may have more than one look: a large visual change of the whole picture, such as the light (a low sun, a high one, the blue hour) or the palette. The scene declares its looks and draws them; the runtime decides when the look changes and to which, following the music, and tells the scene through `struct Look`. A scene without `[[look]]` has none: it works as it always did, and `look` reads zeros. Like the director the looks are per scene and start again at a scene switch (`src/scene/looks.rs`).

**A look is a large visual change, nothing smaller.** A change of what moves or what is drawn (the formation of Torii Sea's spirits, a dance move) is not a look: it stays on a binding or a shot, picked by the section. Look changes are rare (a few in a song, at least 8 s apart) and fade, so a look should be something the eye takes as another light, not another motion.

### What a scene declares

```toml
[[look]]
name = "low sun"     # for people: the lab lists it
tags = ["warm"]      # optional hints for the runtime and the lab; declared only, nothing reads them yet
weight = 1.0         # optional, default 1: how often a kind of section picks it against the others

[[look]]
name = "day"
opens = false        # optional, default true: whether the scene may start in this look

[[look]]
name = "theme colour"
tiers = ["high"]     # optional, default all: the kinds of section, by energy, it suits
at_least_once = true # optional, default false: the song is promised this look (see rule 7)
```

- `weight` is a number, 0 or more: a look of 2 is picked twice as often as one of 1, and one of 0 is never picked while another has a chance. A scene whose looks all have weight 0 is refused.
- `opens = false` keeps the scene from starting in the look (Torii Sea never starts in daylight). A section may still change to it later. A scene whose looks all have `opens = false` is refused.
- `tiers` picks looks by the section's energy: any of `"quiet"`, `"normal"`, `"high"` (the director's tiers, from `director.cadence`'s `below` and `above` on the section's energy, else the music's slow energy). A section picks only among the looks that suit its tier, and among all if none does. Neon Street is grey in its quiet and normal sections and in the theme colour at its climax: a grey look with `tiers = ["quiet", "normal"]` and a colour one with `tiers = ["high"]`. Left out, a look suits every tier.
- `at_least_once` promises the look to a song that has a chart, as for a shot (the same key; Torii Sea's day). Rule 7.
- `tags` are strings and only declared: they are in the manifest for the lab and for a later rule (night first, say), and no code reads them.

### What `struct Look` holds

`look` is available to `update()` and to every pass, on a binding of its own (slot 18, after the inputs). The scene draws the look `prev` and the look `to` and blends them by `mix`, as Growth blended its themes.

| Field | Type, range | Meaning |
|---|---|---|
| `prev` | u32 | The look before this one, as the order of the `[[look]]`s of `scene.toml` from 0. (`from` would be the natural name, but WGSL reserves it.) |
| `to` | u32 | The look now. |
| `mix` | f32, 0 to 1 | How far the fade from `prev` to `to` is, already eased (smoothstep). 1 when no fade is under way. |
| `changes` | u32 | How many changes so far: a new count is a fresh fade. |

At the scene's start `prev` equals `to` and `mix` is 1.

### The rules

They are `look.change` in `runtime.toml` (`when`: `strength` 0.5, `gap` 8 s, `strength_unlabelled` 0, `gap_unlabelled` 30 s, `fade` 1 beat, `fade_seconds` 0.5 s), the same for every scene, and a scene lab can edit them.

1. **A section start changes the look.** A cue of `strength` or more with a chart (a section start) changes the look to the one the section's label picks among those whose `tiers` suit the section's energy: a roll from `rv_label_hash`'s hash of the label, by the looks' `weight`. So every chorus has the same look and a song plays the same each time.
2. **The look on is kept.** If the pick is the look on, nothing changes (and the gap does not start again): the look stays tied to the kind of section.
3. **At least `gap` seconds between two changes.** The scene's start counts as one, so a scene keeps the look it opened in for that long.
4. **Without a chart** there are no labels. A cue of `strength_unlabelled` or more (any cue) changes the look to the next that suits the music's energy in an order made from the scene's seed, at least `gap_unlabelled` seconds after the last change: reproducible, not random.
5. **The fade** lasts `fade` beats with a followed beat, else `fade_seconds`, eased.
6. **The first look** is picked among the looks that `opens`, by weight, from the seed of the scene's first frame.

7. **A look the song is promised (`at_least_once`).** A look that says the key is picked by chance by rules 1 to 3 as any other. The runtime remembers which looks have been on in this song (the scene's first look, and every change) and forgets them when the song time goes back (a new song, or a seek) or the chart's list of sections is replaced; the look on then counts. If a look that says the key has not been on, the last section start that suits it changes to it, in place of the label's pick, as long as the gap allows. The last chance is found as for a shot, from the same list of sections the director has (`ChartState::song`, handed to `Looks::set_song`): the last section that suits the look by its `tiers`, or where none does the song's highest (the last of equals). A section start counts only where `gap` allows a change: the first section, or one that starts at least `gap` after the section before it (a look changes only at section starts, so the change before it is then far enough). If none that suits counts, the song's highest section that does. Without a chart nothing is promised, and a look of the key is only picked by chance.

A scene lab holds a look (`LabCommand::Look(Some(i))`: a change to look `i` with the usual fade, then no more changes) and hands it back with `Look(None)`. A hold stays when the scene's files are saved, if the scene still has as many looks. `Inspect.looks` has the names, the look on, the fade and whether it is held; the timeline marks every change.

### Example

```toml
# scene.toml
[[look]]
name = "low sun"
tags = ["warm"]

[[look]]
name = "blue hour"
tags = ["night"]
weight = 2
```

```wgsl
fn palette(i: u32) -> vec3f { /* the scene's own drawing of look i */ }

fn shade(px: Pixel) -> vec4f {
  let colour = mix(palette(look.prev), palette(look.to), look.mix);
  ...
}
```

The scene never decides a change: it draws `prev`, `to` and `mix`, and reacts to `look.changes` only for what it must restart.

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

An `[effects]` table, in two lists that say which kind each effect is. It is a table, so it goes after the top-level keys, with the others:

```toml
[effects]
# Transitions: switching into this scene, and dressing a hard cut of the camera in it.
transitions = ["fade", "dissolve", "flash", "glitch"]
# Lens effects: over the picture, with no change of picture.
lens = ["flare", "leak", "glitch"]
```

- A **transition** happens at a change of picture: switching into the scene, and dressing a hard cut of the director's camera in it (one list for both). A **lens effect** lies over a picture that goes on. The `glitch` and the `mosaic` exist as both, and the two lists say for which: with one list a scene could not have a mosaic at a switch and not over the picture (Emoji Tunnel: transitions fade, dissolve, cut-in, flash, glitch, mosaic; lens effects flare, leak, glitch).
- `transitions` takes `cut`, `fade`, `dissolve`, `flash` (a colour wash of the theme colour, then overexposure into a light tint of it), `glitch` (rows of broken pieces that shift far, split their colours, invert, streak and flare, in stuttering stages), `mosaic` (big hard blocks: posterized cells in steps, up to the whole frame, then the new picture out of them) and `cutin` (a torn band with the playing cover, after Persona 5's cut-in). `lens` takes `flare`, `leak`, `glitch` (wide tears that jump with the beat) and `mosaic` (many small patches that come and go), each playing over the picture with no cut, to the end of the phrase; `blur` is still to come. (`blocks`, a colour-block frame, is now the start of a `flash`; a scene that lists it gets the warning.)
- Both keys are optional. A name of the other kind is refused, with a message saying where it belongs (`flare` in `transitions`, `flash` in `lens`). A name the runtime does not know is left out with a warning, so a scene written for a newer runtime still loads. The old top-level `effects = [...]` is refused with a message that shows the new table.
- `flash`, `glitch`, `mosaic` and `cutin` are strong; the rest are light.
- Without the table (also MilkDrop presets): a switch fades or dissolves, a cut is a plain cut, no lens effects. `transitions` that names no plain transition (`cut`, `fade`, `dissolve`) keeps the fade and the dissolve for the plain switches. No `lens`, no lens effects.

### When they play: the moments (`src/moments.rs`)

- **Moments.** A section start that lifts the energy (`cue` 0.8 or more, which the director's burst of cuts leads to) is a strong moment. Another section start (`cue` 0.5 or more) and an impact are medium moments. Nothing else is one: no effect on a plain beat or a plain cut.
- **Chance.** A moment plays an effect by the chance of its strength. The user's setting "effects" has two values: default, 40% of the strong moments and 15% of the medium ones (about two effects in a song); more, 60% and 30%. A scene lab may set any chance (it starts at 100%). The roll is a hash of where the moment is in the song (its section, and which impact of it), so a song plays its effects the same way every time. A medium moment does not play within 8 bars of the last effect; a strong one does.
- **Which.** It depends on what happens at the moment. A scene switch (made when a new song starts, see "Scene switching" in `README.md`) takes a transition of the scene coming (the one going has no say): when the new song is silent, a fade through black; otherwise a strong transition the scene declares, picked by the label of the section playing, or a fade if it declares none (a scene that does not take a fade gets a dissolve, then `cut`). A hard cut of the director at a moment that plays is dressed with a transition of `transitions` (`fade`, `dissolve`, `flash`, `glitch`, `mosaic`; a cut-in is not drawn over a cut); a cut to a shot the scene glides to is not dressed and gets only a light lens effect of `lens`, and a strong cut is never one of those. With no cut, or no transition that suits, a lens effect of `lens` plays over the picture (its `glitch` and `mosaic` are strong ones; a glitch or mosaic named only in `transitions` never plays over the picture, and one named only in `lens` never dresses a cut). A strong moment takes a strong effect if the scene declares one, a medium moment only a light one, and the section's label picks among them (the same kind of section, the same effect).
- **The burst's punch.** Every cut of the director's burst gets a short punch whatever the list says (see "Shots and the director").

### Transitions (`src/transition.rs`, `transition.wgsl`)

One transition layer serves the switch from one scene to the next and a cut inside a scene. A switch keeps the old picture (or draws the old scene live for a fade or a flash) and combines it with the new scene; a dressed cut keeps the frame before the cut and plays its course over the scene drawing on after it, with half the durations of a switch so that it stays on the beat. Durations are in beats while the beat is followed: a fade 1 beat out and 1 in, a dissolve 2 beats, a flash 1 beat up and 2 down (the new scene goes in at the peak; the first third of the way up washes the picture in the theme colour, after the reference's colour-block frames: still readable, its luma running from a deep shade of the colour through the colour to a near-white tint, and the rest overexposes the washed picture into the flood; the way down is light falling into the new scene), a mosaic 1 beat, a glitch 1 beat, a cut-in a band of ten frames and a beat of hold over the old scene, which plays on, then a tear of ten frames as the new scene goes in (its cracks held for three, then the strips flying apart).

The glitch and the mosaic hit hard. Over the picture, both land on the moment that starts them: the glitch's first event and the mosaic's first two patches are strong whatever the course has swelled to, since left to the swell the moment passed with nothing for a beat or more.

The glitch of a switch steps rather than moves: its course is a fixed list of stages in equal shares of its time (`glitch_step`), each held for a few frames: it breaks, breaks harder, stutters back to the first stage, then the new picture comes through its heaviest stage and the breaks die down over it (the shares of pieces that break, 55%, 85%, 70%, 95%, 70%, 25%). A shifted piece goes by up to 35% of the width either way, its colour channels torn apart (1 to 4% of the width), some pieces are inverted, and the others streak, go blocky or burn to slabs of light and the theme colour as before.

The mosaic is hard: flat square cells, each the average of the picture under it (nine points), posterized to five levels per channel with a touch of contrast (never full white), no dots and no gaps. A switch is ten steps of its progress (`mosaic_switch` in `transition.wgsl`), a step held for a tenth of it: the old picture in cells of 3%, 5% (both in a region of the frame, the whole frame from the third step), 8%, 10%, 12% of the height; at half way the new picture is behind the largest cells, which step down 12%, 9%, 6%, 3.5%, 2%.

As lens effects (`Overlay`) the glitch and the mosaic play over the live frame alone, like the punch, and take the course of the long effects (see below, `src/course.rs`): to the end of the phrase, their amount rising and falling with the music, their events and patches timed on a cross-rhythm of the linear beat time (see below). One at a time: one started while one of the same kind plays lets it go on, one of the other kind takes its place; a switch ends it. The pass runs only while they play.

- **A glitch over the picture** (`glitch_flow`: not every tear full width with a new layout on every beat, which has no feeling of real randomness; it answers the music like a polyrhythm instead): events on a cross-rhythm, five slots in three beats of the linear beat time (beats since the effect started, `Course::age_beats`; the eased time dwells on the whole beats, so events timed by it would sit on them). A slot hosts an event if its roll is under 30% plus 70% of the amount; it starts up to 0.2 beat into the slot, lasts 0.1 to 0.3 beat, and 3 in 10 have a second, new one 0.2 beat behind. The first event always lands, strong (at least 0.8), at the start. An event is its own random set of blocks, 2 to 8 by the amount: mostly 3 to 33% of the width and 2.5 to 12% of the height, one in eight a long thin tear (55 to 100% by 1 to 3% of the height), at random places. The rest of the frame is the picture. A block is displaced (5 to 25% of the width, the beat's accent adding a little), colour split (1 to 7% between channels), inverted, burnt to the theme colour (its luma in the colour), or a held copy of another region (read from up to 35% across and 25% down). Never past 0.92; nothing lights the whole frame. Seeded by the start, so the same song does the same each time. The glitch of a switch keeps its stages, with narrower pieces (4 to 44% of the width, not up to 70%).
- **A mosaic over the picture** (`mosaic_blocks`: not the full screen but many small patches that appear and disappear round the edges of the frame, each staying its own time, on a cross-rhythm with the music like the glitch): a slot every fifth of a beat of the linear beat time with three chances of a patch, each taken if its roll is under 35% plus 55% of the amount, at a jitter of its own within the slot; the first slot's always land, at the start, strong. A patch sits against one of the four edges (picked by their lengths), at most 12% of the frame's shorter side in from it and mostly nearer; it is 2.5 to 7% of the shorter side tall (a little smaller at a low amount) and 0.8 to 2.2 times as wide, in two or three cells of its own grid across its height (never under 6 pixels), and lives from a quarter beat to two and a half beats, most of them short, so some eight are up at once at a low amount and a dozen at the most. The middle of the frame is left alone (a pixel more than 28% of the shorter side from every edge is not even looked at). Its cells are the average of the picture under them, not posterized (posterized cells read as tiles), never above 0.92. The switch's first two steps are two patches, not a whole-frame region; from the third the whole frame is in cells, as a switch has to be.

### Lens effects in the post-processing (`scene/lens.rs`, `post.wgsl`)

With none listed, the post pass does the same work as before (the effects' passes are not encoded and their branches read a uniform of zeros). Each is started at a moment with a strength (1 at a strong moment, 0.6 at a medium one) and runs its course; none is a constant layer. They are drawn in the app's accent (`frame.accent`; a warm orange before the app tells one) and black and white, and none reaches full white.

Both are laid over the tone-mapped picture in display values, blended like a screen, so they lift the dark instead of greying it.

**The course of the long effects** (`src/course.rs`): the flare, the leak and the glitch and mosaic over the picture all take it.

- **Length by the phrase.** An effect lasts to the end of the phrase it starts in: the next 4-bar boundary counted from the section's start (the bars counted as the camera director counts them, a section start counting again from 0), at least 2 bars away counting what of the bar has gone by, at most 8 bars. Without a followed beat, 4 s.
- **Shape.** From nothing, a smoothstep up to the peak a third of the way through, then a smoothstep down to nothing at the end: never a sudden onset.
- **Amount with the music.** At each frame the shape is multiplied by the music's `intensity`, smoothed over about a beat and floored at 0.35: at still music an effect is at 35% of what it is at full, so it never vanishes mid-way; the glitch breaks more blocks and the mosaic puts up more patches, a little bigger, when the music is full, and fewer and smaller when it holds back.
- **Moving on the beat, eased.** What moves in an effect (the flare's and the leak's light) moves, and what steps in one (the glitch's tears and the mosaic's patches), steps on an eased beat time (the glitch and the mosaic, which are timed on a cross-rhythm, use the linear one instead): the whole beats as they are, each eased within itself (`floor(t) + ease(fract(t))`, an ease-out `1 - e^(-10x)` scaled to end at 1), so it is continuous and never jumps, but pushes on at every beat, 86% of a beat's motion in its first fifth, and settles before the next (stepping on the beat was jerky and a smooth clock was limp). Its beat time is in phase with `clock` while the beat is followed and runs at the tempo; without a followed beat each hit pushes it on to the next whole beat, which it runs to as a beat of 0.5 s does. Every beat (or hit) also gives an accent that rises within about two frames at 120 bpm and settles over the beat, as strong as the music's `hit` then, at least 30% of a full one, so strong beats push harder: the glitch's displaced blocks go a little further (the mosaic does not use it).
- **Again.** A flare or a leak started while one runs carries on from the level it has, back on the rising side, with the new phrase's length, so it swells again without a jump; two never add up. A glitch or a mosaic of the same kind goes on as it is.

- **`flare`**: after Keijiro Takahashi's Kino Streak and John Chapman's pseudo lens flare. The highlights in the exposed HDR picture, above a threshold of 1 (with a soft knee of 30 % on each side) raised to 1.5 times the brightness round each point (a ring 6 % of the height off), so that only what stands out from its surroundings flares and a wide bright sky does not (Torii Sea's low sun washed the whole picture white before), the excess over it eased towards 4 so a sun draws a streak only a little longer than a lamp, and anything not finite dropped. The streaks lean: each flare takes an angle of up to 40 degrees either way from the horizontal, picked by the section's label with the song's analysis (the same kind of section flares the same way, a song the same way each time), and without it (an online track, the first seconds) by how many flares came before, so each leans another way, kept while it swells again; it is drawn on a canvas turned by that angle and scaled to cover the corners (a streak always level is dull) are smeared into thin anamorphic streaks by a pyramid that shrinks only across (each level half the width of the one before, a quarter of the internal width and half its height to start, down to a few texels), added back level by level, each wider level weighing less. The streaks are in the theme colour going nearly white where they are bright. Faint ghosts and a halo are sampled from the highlights mirrored through the centre, faded and fringed in colour towards the edges, under a faint fixed lens dirt. They read the highlights averaged over a disc 2 % of the picture's height in radius (two passes at an eighth of the width and height, square texels, so the disc is round whatever the shape of the highlights' texels, the aspect or the angle), so a light of one texel leaves a soft round disc and a large light a soft blob (the first version read them a texel off and a small light left a hard block, mirrored in a pair); a small light's ghost is dimmer than its streak, as the light is spread over the disc. Nothing is tracked and nothing adapts to the frame; the highlights are averaged with the frame before's, so one that shimmers about the threshold glows steadily. Its intensity (0.8 at the peak at full intensity of the music) and the reach of its streaks follow its course. It is drawn only from `frame.quality` 0.5 up.
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
