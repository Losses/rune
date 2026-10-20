# Optional built-in preset pack

The visualizer can ship with a built-in MilkDrop preset pack ("MilkDrop Original": 552 presets and 67 textures). **It is off by default**: a plain `cargo build` or `flutter build` embeds nothing, `builtin_pack()` returns `None`, and the app shows no source switcher.

## How it is switched on

`visualizer/build.rs` reads the environment variable `RUNE_BUILTIN_PRESETS`. When it names an existing zip file, the file is copied to `OUT_DIR` and embedded with `include_bytes!`, and the build enables `cfg(builtin_presets)`. The variable and the file are tracked with `rerun-if-env-changed` and `rerun-if-changed`, so toggling it rebuilds the crate.

The zip layout is:

```
presets/<name>.milk      the presets (also .milk2)
textures/<name>.jpg      the textures (any plain texture extension works)
SOURCES.txt              commit SHAs and the LICENSE/README notices of both repositories
```

The zip is built from these repositories, pinned to the commit it records in `SOURCES.txt`:

- <https://github.com/projectM-visualizer/presets-milkdrop-original>
- <https://github.com/projectM-visualizer/presets-milkdrop-texture-pack>

A build embeds the zip that `RUNE_BUILTIN_PRESETS` points at:

```sh
RUNE_BUILTIN_PRESETS=/path/to/milkdrop-original.zip cargo test -p visualizer
```

## Behaviour

- Before any preset is chosen, and in builds without the pack, the visualizer shows Rune's own preset, `visualizer/presets/Rune.milk` (written for Rune, MPL-2.0 like the rest of it). `cargo run -p visualizer --example snapshot -- <dir>` renders it (or any `.milk`) to images.

- With a build that has the pack and no preset folder chosen, the visualizer starts with the pack and a random preset. With a folder chosen, it starts with the folder. The switcher in the controls (shown only when the pack is available) changes the source, and the choice is saved in the settings (`visualizer_source`).
- Only the presets directly in `presets/` take part in switching (475 of the 552). The 77 in `presets/affected/` are in the pack but never shown by next, previous, random or the automatic switching. A folder source works the same way: only the `.milk` / `.milk2` files directly in the chosen folder count, subfolders are not searched.
- Presets are read from the embedded zip on demand. Textures are decoded on first use and cached up to 64 MiB; the pack is never decoded up front.
- A folder source uses its `textures` subfolder for textures if it has one, otherwise the folder itself, with lactea-host's `DirTextures` lookup rules.

## Preset switching

The rules follow MilkDrop 2's defaults (Nullsoft, BSD-3), for the pack and for a folder alike:

- Random picks are weighted by each preset's `fRating=` line (missing or unreadable: 3, clamped to 0..5). A preset rated 0 is never picked; if all are 0, the pick is uniform. Ratings are read when the list is built and never written.
- The last 64 loaded presets are kept. Previous walks back through them (and does nothing at the start). Next walks forward again after going back, otherwise it loads a new random preset. Random always loads a new one and drops the way forward.
- The visualizer switches by itself 2.7 s (blend) + 16 s + 0..10 s (random) after the last load, with a 2.7 s blend. Manual switches blend for 1.7 s. The wait starts over after every load. While the preset is locked (context menu) or the context menu is open, the wait keeps starting over.
- Hard cuts on beats are off, as in MilkDrop 2.

## Licensing caveat

The presets and textures have no license of their own, and neither repository contains a license file (see `SOURCES.txt` in the zip for what they do contain). projectM's preset collections state its position in their `LICENSE.md` (for example [presets-cream-of-the-crop](https://github.com/projectM-visualizer/presets-cream-of-the-crop/blob/master/LICENSE.md) and presets-en-d): presets "were, in almost all cases, not released under any specific license. Theoretically, each preset author holds the full copyright on any released presets", and since they were freely released and widely used "it is safe to assume them to be in the public domain"; authors can ask for removal. That is an assumption, not a license grant. Because of that, **upstream Rune must not include them by default**; they are only compiled in when the person building opts in with `RUNE_BUILTIN_PRESETS`, and they are never committed to this repository. Anyone who distributes a build made with the pack takes that decision themselves.
