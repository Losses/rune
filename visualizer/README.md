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
