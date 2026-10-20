# Visualizer presets

Rune ships no MilkDrop presets of its own except one default preset. Everything else comes from a folder the user picks.

## Where presets come from

- The default preset is `visualizer/presets/Rune.milk`, written for Rune (MPL-2.0 like the rest of it). The visualizer shows it when no preset folder is set, and again when the user stops using the folder. `cargo run -p visualizer --example snapshot -- <dir>` renders it (or any `.milk`) to images.
- A preset folder is chosen in Settings > Theme. Its `.milk` and `.milk2` files, also those in its subfolders, take part in next, previous, random and the automatic switching. Hidden folders (names starting with `.`) and symlinked folders are skipped. Without a folder, next, previous and random do nothing.
- Clearing the folder empties the list and the history, and a running visualizer softly (1.7 s) changes to the default preset.
- Textures (`sampler_<name>`, `rand##`) are looked up relative to the chosen folder, not per subfolder: in `<folder>/textures`, else in `<folder>` itself if it holds images, else in `<folder>/../textures` (a folder laid out like Winamp's, with the presets in a subfolder and `textures` next to it). The lookup rules are those of lactea-host's `DirTextures`. Textures are decoded on first use and cached up to 64 MiB.

## Preset switching

The rules follow MilkDrop 2's defaults (Nullsoft, BSD-3):

- Random picks are weighted by each preset's `fRating=` line (missing or unreadable: 3, clamped to 0..5). A preset rated 0 is never picked; if all are 0, the pick is uniform. Ratings are read when the list is built and never written.
- The last 64 loaded presets are kept. Previous walks back through them (and does nothing at the start). Next walks forward again after going back, otherwise it loads a new random preset. Random always loads a new one and drops the way forward.
- The visualizer switches by itself 2.7 s (blend) + 16 s + 0..10 s (random) after the last load, with a 2.7 s blend. Manual switches blend for 1.7 s. The wait starts over after every load. While the preset is locked (context menu) or the context menu is open, the wait keeps starting over.
- Hard cuts on beats are off, as in MilkDrop 2.
