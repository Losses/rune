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

- The names are the runtime's. Transitions: `cut`, `fade`, `dissolve`, `flash` (overexposure into a light tint of the theme colour), `glitch` (a datamosh smear), `mosaic` (LED dot blocks), `cutin` (a torn band with the playing cover, after Persona 5's cut-in). Lens effects: `flare`, `leak`, `blocks` (a colour-block frame); `blur` is still to come. A name the runtime does not know is left out with a warning, so a scene written for a newer runtime still loads.
- `flash`, `glitch`, `mosaic`, `cutin` and `blocks` are strong; the rest are light.
- Without the list (also MilkDrop presets): a switch fades or dissolves, a cut is a plain cut, no lens effects. A list that names no plain transition (`cut`, `fade`, `dissolve`) keeps the fade and the dissolve for the plain switches.

### When they play: the moments (`src/moments.rs`)

- **Moments.** A section start that lifts the energy (`cue` 0.8 or more, which the director's burst of cuts leads to) is a strong moment. Another section start (`cue` 0.5 or more) and an impact are medium moments. Nothing else is one: no effect on a plain beat or a plain cut.
- **Chance.** A moment plays an effect by the chance of its strength. The user's setting "effects" has two values: default, 40% of the strong moments and 15% of the medium ones (about two effects in a song); more, 60% and 30%. A scene lab may set any chance (it starts at 100%). The roll is a hash of where the moment is in the song (its section, and which impact of it), so a song plays its effects the same way every time. A medium moment does not play within 8 bars of the last effect; a strong one does.
- **Which.** It depends on what happens at the moment. A scene switch takes a transition of the scene coming (the one going has no say): at a lift whose moment plays, a strong transition the scene lists, picked by the label of the section it lands on; otherwise a lift or a plain change fades and a drop dissolves (a scene that takes neither gets the other, then `cut`). A cut of the director at a moment that plays is dressed with a transition of the list (`fade`, `dissolve`, `flash`, `glitch`, `mosaic`; a cut-in is not drawn over a cut). With no cut, or none that suits, a lens effect of the list plays over the picture. A strong moment takes a strong effect if the scene lists one, a medium moment only a light one, and the section's label picks among them (the same kind of section, the same effect).
- **The burst's punch.** Every cut of the director's burst gets a short punch whatever the list says (see "Shots and the director").

### Transitions (`src/transition.rs`, `transition.wgsl`)

One transition layer serves the switch from one scene to the next and a cut inside a scene. A switch keeps the old picture (or draws the old scene live for a fade or a flash) and combines it with the new scene; a dressed cut keeps the frame before the cut and plays its course over the scene drawing on after it, with half the durations of a switch so that it stays on the beat. Durations are in beats while the beat is followed: a fade 1 beat out and 1 in, a dissolve 2 beats, a flash 1 beat up and 2 down (the new scene goes in at the peak), a mosaic 1 beat, a glitch 0.15 s, a cut-in a band of ten frames and a beat of hold over the old scene, which plays on, then a tear of ten frames as the new scene goes in (its cracks held for three, then the strips flying apart). A switch that knows its section start (`cue_in`) starts early, so that the new scene shows on it.

### Lens effects in the post-processing (`scene/lens.rs`, `post.wgsl`)

With none listed, the post pass does the same work as before (the effects' passes are not encoded and their branches read a uniform of zeros). Each is started at a moment with a strength (1 at a strong moment, 0.6 at a medium one) and runs its course; none is a constant layer. They are drawn in the app's accent (`frame.accent`; a warm orange before the app tells one) and black and white, and none reaches full white.

The flare and the block frame start from the picture at a quarter of the internal size (exposed, filtered so that no halftone dot or lone bright pixel is left) and the frame's key: its brightest luma, its mean and where the brightest is, from 8 by 8 cells, followed smoothly from frame to frame. So they find a night scene's highlights and shapes as well as a bright scene's. All three are laid over the tone-mapped picture in display values (the flare and the leak blended like a screen, so they lift the dark instead of greying it).

- **`flare`**: long thin anamorphic streaks along the rows of the picture's highlights (those from a little over halfway between the frame's mean and its brightest; the brightest throws a full streak whatever the scene's brightness), a sharp core nearly white with soft tails in the theme colour, and four ghosts with a fringed rim along the line from the brightest highlight through the centre. As it starts, a bright thin streak sweeps out from the brightest highlight to the sides over a beat; the glare comes in over a quarter of a beat and goes over six (a bar and a half). Two passes at a quarter of the size draw it. It is drawn only from `frame.quality` 0.5 up.
- **`leak`**: a film light leak: a large soft glow (its radius half the frame's long side) from beyond a corner, its outline torn by low-frequency noise, saturated in the theme colour going warmer towards its source. It blooms in over a beat with a hot burn at its edge that is gone two beats later, drifts along its edge, and goes over the next 16 beats.
- **`blocks`** (the colour-block frame): a graphic frame after the reference video's: the picture posterised to three flat tones of the theme colour (the shadows a deep shade, most of it the colour, the highlights a light tint), with white line art of even width along the boundaries of the tones. Two frames of it at 60 fps, then one of its negative (the tones swapped, the lines dark); each drawn at least once.

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
