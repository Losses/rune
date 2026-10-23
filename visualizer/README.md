# Visualizer presets

Rune ships no MilkDrop presets of its own except one default preset. Everything else comes from folders the user picks.

## Where presets come from

- The default preset is `visualizer/presets/Rune.milk`, written for Rune (MPL-2.0 like the rest of it). The visualizer shows it when no preset folder is set, and again when the user removes the last one. `cargo run -p visualizer --example snapshot -- <dir>` renders it (or any `.milk`) to images.
- Preset folders are added and removed in Settings > Theme, any number of them. Their `.milk` and `.milk2` files, also those in their subfolders, take part in next, previous, random and the automatic switching, once per file even if folders overlap. Hidden folders (names starting with `.`) and symlinked folders are skipped. A folder that cannot be read is skipped and reported in the status error. Without a folder, next, previous and random do nothing.
- Setting the folders (`Visualizer::set_preset_dirs`) empties the list and the history. With no presets left, a running visualizer softly (1.7 s) changes to the default preset; otherwise the app asks for a random preset.
- Textures (`sampler_<name>`, `rand##`) are looked up relative to each chosen folder, not per subfolder, folders in their order and the first match wins: in `<folder>/textures`, else in `<folder>` itself if it holds images, else in `<folder>/../textures` (a folder laid out like Winamp's, with the presets in a subfolder and `textures` next to it). The lookup rules are those of lactea-host's `DirTextures`. Textures are decoded on first use and cached up to 64 MiB.

## Preset switching

The rules follow MilkDrop 2's defaults (Nullsoft, BSD-3):

- Random picks are weighted by each preset's `fRating=` line (missing or unreadable: 3, clamped to 0..5). A preset rated 0 is never picked; if all are 0, the pick is uniform. Ratings are read when the list is built and never written.
- The last 64 loaded presets are kept. Previous walks back through them (and does nothing at the start). Next walks forward again after going back, otherwise it loads a new random preset. Random always loads a new one and drops the way forward.
- The visualizer switches by itself 2.7 s (blend) + 16 s + 0..10 s (random) after the last load, with a 2.7 s blend. Manual switches blend for 1.7 s. The wait starts over after every load. While the preset is locked (context menu) or the context menu is open, the wait keeps starting over.
- Hard cuts on beats are off, as in MilkDrop 2.

## Writing presets

The preset on screen is reloaded (with a cut) whenever its file is saved, so a preset can be written in any editor while it plays. Put it in a folder of its own, add that folder in Settings > Theme, and lock the preset from the cover wall's context menu so the automatic switching leaves it alone. A save that does not load, or whose shaders do not compile (lactea then draws with a default shader), shows the problem in the visualizer's corner until the next save fixes it. `cargo run -p visualizer --example snapshot -- <dir> <preset.milk>` renders a preset to images without the app.

## The beat, for presets

MilkDrop tells presets how loud three bands are, relative to their recent average, and nothing about tempo. Rune also runs a beat tracker on the music (`src/rhythm.rs`) and sets these variables before every frame's per-frame code (and before the init code of a preset it loads). A preset that does not use them is not affected, and elsewhere they read 0, so a preset can fall back on `bass` when `rune_beat_conf` is low.

| Variable | |
| --- | --- |
| `rune_bpm` | The tempo, 0 until one is found. It may be double or half the tempo a listener would tap. |
| `rune_beat_count`, `rune_beat_phase` | A beat clock: whole beats since the visualizer started, and the phase within the beat (0 on the beat, rising to 1). `rune_beat_count + rune_beat_phase` only grows, so a preset can shift it ahead (`+ seconds * rune_bpm / 60`) to make up for latency and still count beats by its whole part. The clock keeps time through breaks. |
| `rune_beat_conf` | 0 to 1: how clearly the music has a beat the clock follows. Low in silence, rubato and music without drums. It cannot tell an off-beat lock from the beat. |
| `rune_kick`, `rune_snare`, `rune_hat` | Onsets of three bands (30 to 160 Hz, 160 Hz to 4 kHz, 5 to 16 kHz), 0 to 1, 1 on a hit as strong as the strongest of the last few seconds, falling within about 0.1 s. They follow the sound, about 25 ms late. |
| `rune_energy` | Loudness of the last second against the last 20 seconds: about 1, more in a chorus or a drop, less in a break, 0 in silence. |

Only the per-frame code sees them. Pass what the shaders need through `q` variables. `cargo run -p visualizer --example beat_track -- <rate> <channels>` runs the tracker over raw `f32` samples from stdin.

## The song's chart

A chart is what an analysis of the whole file found: its beats, bars, sections and onsets. The player tells the visualizer which file plays and where (`Visualizer::set_playback`, with every progress report), and the visualizer looks for the file's chart in the chart folder (`RUNE_CHART_DIR`, else `rune/charts` in the user's cache folder, `~/Library/Caches` on macOS), named after a hash of the path (`chart_key`). A Python chart analyzer writes them. `cargo run -p visualizer --example chart_at -- <audio file> [seconds...]` prints what presets would see.

With a chart, `rune_bpm`, `rune_beat_count`, `rune_beat_phase` and `rune_beat_conf` come from its beats (the count is then beats since the song's first beat, and jumps on a seek), and these are set too. Without one, they read 0.

| Variable | |
| --- | --- |
| `rune_chart` | 1 while the playing file has a chart. |
| `rune_bar_count`, `rune_bar_phase` | Bars since the first, and the phase within the bar (0 on the downbeat). |
| `rune_beats_per_bar`, `rune_meter_conf` | The meter (3 or 4), and 0 to 1: how sure the analysis is of where bars start. |
| `rune_section`, `rune_section_label`, `rune_section_repeat` | Index of the section, its label (0 for the first kind of section, 1 for the next new kind; the same label for sections that sound alike) and how many earlier sections had that label. |
| `rune_section_energy`, `rune_next_label`, `rune_next_energy` | Loudness of this section and the label and loudness of the next one, 0 to 1 within the song. |
| `rune_section_time`, `rune_section_left`, `rune_section_conf` | Seconds since the section started and until the next one, and 0 to 1: how clear the change into it was. |
| `rune_ev_low`, `rune_ev_mid`, `rune_ev_high`, `rune_ev_harm`, `rune_ev_phrase` | Onsets of the percussive low, mid and high bands, of pitched sounds and of phrases: the latest one's strength, fading within about 0.1 s (0.6 s for phrases). |
| `rune_lead`, `rune_lead_ev` | The stream that stands out now (0 none, 1 low, 2 mid, 3 high, 4 pitched) and its onsets. |
| `rune_loud`, `rune_bright`, `rune_perc`, `rune_pulse` | Loudness, brightness, how percussive the sound is and how clear the pulse is, 0 to 1 within the song. |
