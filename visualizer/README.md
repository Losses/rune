# Visualizer scenes

The visualizer draws scenes in Rune's own format and nothing else. Rune ships one, Armillary, compiled into the binary. Everything else comes from folders the user picks.

## Where scenes come from

- The built-in scene is `visualizer/scenes/armillary` (MPL-2.0 like the rest of Rune), embedded with `include_str!`. The visualizer shows it when no scene folder is set, and again when the user removes the last one. An embedded scene is read from memory and never reloads. It is used for the light and the dark appearance alike.
- Scene folders are added and removed in Settings > Theme, any number of them. A folder inside them with a `scene.toml` is one scene; the folders in between, also subfolders, are searched, and a scene's own contents are not. All take part in next, previous, random and the automatic switching, once per scene even if folders overlap. Hidden folders (names starting with `.`) and symlinked folders are skipped. A folder that cannot be read is skipped and reported in the status error. Without a folder, next, previous and random do nothing.
- Setting the folders (`Visualizer::set_preset_dirs`) empties the list and the history. With no scenes left, a running visualizer changes to the built-in scene; otherwise the app asks for a random scene.

## Scene switching

The rules follow MilkDrop 2's defaults (Nullsoft, BSD-3), without the ratings:

- Random picks are uniform. The last 64 loaded scenes are kept. Previous walks back through them (and does nothing at the start). Next walks forward again after going back, otherwise it loads a new random scene. Random always loads a new one and drops the way forward.
- The visualizer switches by itself 2.7 s + 16 s + 0..10 s (random) after the last load. The wait starts over after every load. While the scene is locked (context menu) or the context menu is open, the wait keeps starting over. Every switch is a cut.

## Writing scenes

The scene on screen is reloaded whenever its files change, so a scene can be written in any editor while it plays. Put it in a folder of its own, add that folder in Settings > Theme, and lock the scene from the cover wall's context menu so the automatic switching leaves it alone. A save that does not load shows the problem in the visualizer's corner until the next save fixes it.

## Scenes in Rune's own format

A scene is a folder with a `scene.toml` (nothing inside it is scanned for other scenes). A scene is written in WGSL only. `update()` runs once per frame on the GPU and keeps whatever `struct State` declares. Its passes draw into HDR buffers that can keep the last frame. The runtime adds bloom, AgX tone mapping, chromatic aberration, vignette, grain and dithering. The scene reads the music as named fields: beat, bar and section with how many of each went by since the last frame, the chart's energy, onsets and leading stream, a spectrum and the waveform. Parameters with labels, ranges and units are declared in `scene.toml` and appear as `params.<id>`. `frame.accent` is the app's accent colour (which follows the system, the user's choice or the cover), so a scene can take its colours from it. A scene that declares `covers = true` in `scene.toml` also sees album covers from the user's library: `covers` is a 1024x1024 texture (sRGB, with mips) of 8x8 cells of 128 px, row-major from the top left, each cover scaled to fill its cell (the middle square of it). Cell 0 is the cover of the track playing now and stays transparent if it has none, cells 1.. are random covers; `frame.covers` is how many cells from the first are valid (0 when the library has no covers, else at least 1; cell 0 counts even when empty). The scene sees one clear texel and `frame.covers == 0` until the first covers are loaded. The random covers are looked up and decoded in the background when such a scene loads and stay while the track changes; then only cell 0 is replaced (also in the background), so nothing blocks a frame and covers do not all change at once; other scenes cost nothing.

- Images are `[[asset]]`s (PNG or JPEG, `asset_<id>` in WGSL, with mips). `linear = true` marks data such as a distance field, which must not be decoded as sRGB.
- Code that several passes share goes in files listed as `common = [...]` in `scene.toml`; every pass includes them (the update does not).
- `src/scene/manifest.rs` is the reference for `scene.toml`, `src/scene/wgsl.rs` for what every shader sees (the prelude), and `src/scene/stdlib.wgsl` for the `rv_*` functions.
- The scene on screen reloads when its files change (the built-in one never does). When only parameter defaults changed, nothing else is rebuilt. When `struct State` reads the same, the state carries over. Errors name the file and line.
- Switching between scenes cuts.
- `cargo run -p visualizer --example snapshot -- <dir> [scene folder]` renders a scene to images, the built-in one without a folder. `SNAPSHOT_SIZE=1920x1080` sets the size, and the frame rate is printed.

## The beat

Rune runs a beat tracker on the music (`src/rhythm.rs`) and hands a scene its findings as named fields: the tempo, a beat clock, how clearly the music has a beat, onsets of three bands (30 to 160 Hz, 160 Hz to 4 kHz, 5 to 16 kHz) and the loudness of the last second against the last 20 seconds. `cargo run -p visualizer --example beat_track -- <rate> <channels>` runs the tracker over raw `f32` samples from stdin.

## The song's chart

A chart is what an analysis of the whole file found: its beats, bars, sections and onsets. The player tells the visualizer which file plays and where (`Visualizer::set_playback`, with every progress report), and the visualizer looks for the file's chart in the chart folder (`RUNE_CHART_DIR`, else `rune/charts` in the user's cache folder, `~/Library/Caches` on macOS), named after a hash of the path (`chart_key`). A Python chart analyzer writes them. `cargo run -p visualizer --example chart_at -- <audio file> [seconds...]` prints what scenes would see.

With a chart, the tempo, beat clock and beat confidence come from its beats (the count is then beats since the song's first beat, and jumps on a seek), and the scene also sees bars, the meter, sections (index, label, loudness, time in and left, what comes next), onsets of the percussive low, mid and high bands, pitched sounds and phrases, the leading stream, and loudness, brightness, percussiveness and pulse. Without one, these read 0. `src/scene/inputs.rs` is the reference for the fields.

## How frames reach the screen

The render thread draws each frame on the GPU, and the app's native side hands it to Flutter as a texture. The status reports the way it took (`transport`) and the adapter it renders on.

- macOS (`iosurface`): frames are drawn straight into IOSurface-backed pixel buffers, which Flutter shows without a copy.
- Windows (`dxgi-shared`): frames are drawn straight into DirectX 12 textures shared through NT handles, which Flutter opens on its own device (through ANGLE) without a copy. That only works on the adapter Flutter renders on, so the app tells the visualizer that adapter's LUID before it starts, and the visualizer renders there. The app registers a second texture for read-back frames and shows whichever matches the transport.
- Linux (`vulkan-shared`): frames are drawn straight into Vulkan images whose memory is exported as file descriptors, which the app imports into Flutter's OpenGL context (`GL_EXT_memory_object_fd`) as textures without a copy. That only works on the GPU and driver of that context, so the app reads their UUIDs from a context like Flutter's before the visualizer starts, and the visualizer renders on the Vulkan device with the same UUIDs. After each frame the image is released to the external queue family and a semaphore of its own is signalled, which the app waits for (`GL_EXT_semaphore_fd`) before Flutter samples the frame. As on Windows, the app registers a second texture for read-back frames and shows whichever matches the transport.
- The fallback everywhere (`readback`): frames are drawn into an offscreen texture and copied into a buffer that is mapped for reading; Flutter uploads the pixels again. With rows that are a multiple of 64 pixels wide, the native side hands over the mapped buffer without copying it on the CPU.

Frames are read back instead when `RUNE_VISUALIZER_READBACK=1` is set (or a test asks for it), when the zero-copy targets cannot be created, or when they fail while running (the switch shows in the status). On Windows also when the app could not tell Flutter's adapter, or when no DirectX 12 adapter has its LUID; the visualizer then lets wgpu pick an adapter, preferring a high-performance one, as on the other platforms. On Linux also when Flutter renders in software (`FLUTTER_LINUX_RENDERER=software`), when its OpenGL context lacks `GL_EXT_memory_object_fd` or `GL_EXT_semaphore_fd` or runs on several GPUs, when no Vulkan adapter has its device and driver UUIDs, when that adapter cannot export memory and semaphores as file descriptors (or has no Vulkan 1.1), and, from then on until the app restarts, once OpenGL failed to import a frame.

The zero-copy paths keep the last three published frames, and every frame someone still holds, away from the renderer, because Flutter may sample a frame after it let go of it. With every target in use the renderer skips a frame instead of drawing over one.
