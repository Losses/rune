# The scene format

A scene is a folder: a `scene.toml` that says what the scene is made of, and WGSL files that draw it. This document is the reference for the format itself: what the folder holds, every key of `scene.toml`, and everything the scene's WGSL gets and must provide. `src/scene/manifest.rs` parses `scene.toml`, `src/scene/wgsl.rs` builds the shader modules and `src/scene/stdlib.wgsl` is the library; where this document and the code differ, the code is right, and this document is wrong.

What the music means, and how a scene takes it, is in [`music.md`](music.md). It owns the signals, the bindings (`[[input]]`, `[[bind]]`), the camera director (`[camera]`, `[[shot]]`, `[[burst]]`), the looks (`[[look]]`), the shared effects (`[effects]`), the parameters for tuning and the performance budget. This document lists their keys, so that the whole manifest is in one place, and leaves their meaning to the sections it links. `README.md` says how scenes reach the screen, how the folders are scanned, and how frames are paced.

## What a scene is

A scene is a folder with a file called `scene.toml` in it. That file is what makes the folder a scene. The rest of the folder is whatever `scene.toml` points at, by paths inside the folder:

- the WGSL files: one for `struct State`, one for `update()`, any number of `common` files, one per pass;
- images, as `[[asset]]`s (PNG or JPEG).

A path is relative to the scene's folder and goes only downward: no `..`, no leading `/`, no drive. Every file must be UTF-8 text or an image as above.

**Built-in scenes** are the folders of `visualizer/scenes/<kebab-name>/`, such as `visualizer/scenes/torii-sea/`. `build.rs` embeds every file of every such folder into the program, except hidden files and a top-level `tools/` folder (the scripts that made an asset stay in the repository and out of the program). A scene is added to the program by adding a folder with a `scene.toml` there. Built-in scenes are MPL-2.0 like the rest of Rune.

**A user's scenes** are the folders the user adds in Settings > Theme. The scan looks through each folder and its subfolders for a folder that holds a `scene.toml`; what is inside a scene is not scanned for more scenes, and hidden folders and symlinks are skipped. A scene found this way is loaded from disk and reloads whenever its files change.

**Names.** A built-in scene is known by its `name` (`"Torii Sea"`) and chosen as `builtin:<name>`. A folder scene is listed under its folder's name, not its `name`; a folder scene with the same name as a built-in one does not replace it: both are listed, the built-in one without a group.

## `scene.toml`

The file is TOML. Keys written before the first table header are the top-level keys. Every key and table below is what the parser accepts, with its type, its default and what it refuses. "Required" means the file is refused without it.

### Top-level keys

| Key | Type | Default | Meaning |
|---|---|---|---|
| `format` | integer | required | The version of this format. `1`. See "Versioning". |
| `name` | string | required | The scene's name, for people. Not empty or blank. |
| `license` | string | required | The scene's licence, free text (`"MPL-2.0"`). Not empty or blank. |
| `author` | string | none | Who made it. |
| `description` | text | none | What the scene shows, for people. A plain string, or a table of one string per language: `{ en = "...", de = "..." }`. A reader takes its language, else `en`, else any. |
| `common` | array of paths | `[]` | WGSL files that every pass includes. The update does not. |
| `covers` | bool | `false` | Whether the scene reads the `covers` texture. See "Textures and samplers". |

The tables, in the order the sections below take them: `[state]`, `[render]`, `[[buffer]]`, `[[pass]]`, `[[asset]]`, `[[param]]`, `[post]`, then the five that `music.md` owns: `[[input]]` and `[[bind]]`, `[camera]`, `[[shot]]` and `[[burst]]`, `[[look]]`, `[effects]`.

Limits: at most 16 passes, 8 buffers, 4 assets, 32 inputs and 4 bursts. A scene needs at least one pass, and at least one pass draws into `out`.

### Names

The `id` of a parameter, a buffer, an asset and an input is a name the shaders see. It is ASCII letters, digits and `_`, does not start with a digit, and does not start with `rv_` (those names are the runtime's). It must be unique among all four kinds together: a parameter and an input cannot share a name even though one is read as `params.x` and the other as `input.x`. A name must also be legal WGSL: WGSL reserves words such as `from` and `target`, and the parser does not know them, so a scene that uses one is refused later, by the shader check, with the file and line.

| Wrong | Error |
|---|---|
| `id = "1x"` or `"a-b"` | `"1x" is not a valid name` |
| `id = "rv_speed"` | `rv_speed: names starting with rv_ are the runtime's` |
| the same id twice | `speed is declared twice` |
| a path with `..` or a leading `/` | `"../x.wgsl" is not a path inside the scene` |

### `[state]`

Declares the scene's state and its update. Without the table, a scene has neither.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `file` | path | required | The file that declares `struct State`. Every shader includes it. |
| `update` | path | none | The file that declares `fn update()`, run once a frame. |

A `[state]` without `file` is refused (`missing field file`); `update` needs the table, so a scene cannot have an update without a state file. See "The update and the state".

### `[render]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `scale` | float | `1.0` | The internal resolution against the output's. It is the most the scene draws at: the runtime lowers it to half of it when frames take too long (see [Performance](music.md#performance)). Above 0 and at most 2, else `render.scale must be above 0 and at most 2`. |

### `[[buffer]]`

A buffer is a texture that passes draw into and later passes read. Declare one for each intermediate picture.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `id` | name | required | The buffer's name. `out` is the built-in target and cannot be declared (`out is the built-in target and cannot be declared`). |
| `format` | word | `rgba16float` | `rgba16float`, `rgba32float`, `rgba8unorm`, `r16float` or `rg16float`. Another word is refused with the list. |
| `scale` | float | `1.0` | The size against the internal resolution. Above 0 and at most 2, else `<id>: scale must be above 0 and at most 2`. |
| `persistent` | bool | `false` | Keeps the last frame: the buffer is two textures that take turns, and a pass reads the older one as `prev:<id>`. |
| `mips` | bool | `false` | A mip chain is made after each write, for blurred reads. `rgba32float` cannot be filtered and so cannot have mips: the scene is refused when it is built (`<id>: rgba32float cannot be filtered, so it cannot have mips`). |

All the buffers together may take 256 MB at the scene's own scale (a persistent buffer counts twice, mips a third more), else `the buffers need N MB at this size, more than 256 MB`.

### `[[pass]]`

The passes draw, in the order they are listed, one full-screen shader each.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `shader` | path | required | The WGSL file of the pass. It must define `shade`. |
| `target` | word | required | The `id` of a buffer, or `out`. |
| `inputs` | array of words | `[]` | The buffers the shader reads: `<id>` for the buffer as this frame's earlier passes drew it, `prev:<id>` for the last frame of a persistent buffer. |

Errors name the pass by its place and file, as `pass 2 (final.wgsl): ...`:

| Wrong | Error |
|---|---|
| a target or input that is not a buffer | `no buffer <id>` |
| `prev:x` of a buffer that is not persistent | `prev:x needs a persistent buffer x` |
| an input equal to the pass's own target | `reads the buffer it draws into (use prev:x for the last frame)` |
| no pass has `target = "out"` | `no pass draws into out` |
| no pass at all | `the scene has no pass` |

`out` cannot be an input: it is what the post-processing reads. The parser does not check that a buffer is drawn before it is read; a pass that reads a buffer a later pass draws reads what that buffer held after the last frame's draw.

### `[[asset]]`

An image the scene draws with.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `id` | name | required | The WGSL name is `asset_<id>`. |
| `file` | path | required | A PNG or JPEG in the scene's folder. Any size. |
| `source` | string | none | Where the image comes from and under what licence, as it must be credited. The app lists the `source` of every built-in scene's assets in its credits (About). Write it as the licence asks, with the changes made. |
| `linear` | bool | `false` | The pixels are data (a distance field, a height map), not sRGB colours: they are not decoded as sRGB. |

The runtime decodes it to RGBA8 and makes the whole mip chain, so an image seen small does not shimmer. An image that cannot be decoded refuses the scene (`Failed to decode <file>`).

### `[[param]]`

A value of the scene that the user tunes. The scene reads it as `params.<id>`. How parameters are chosen and named is in [Parameters for tuning](music.md#parameters-for-tuning).

| Key | Type | Default | Meaning |
|---|---|---|---|
| `id` | name | required | The WGSL name, `params.<id>`. |
| `type` | word | `float` | One of the five below. |
| `default` | by `type` | required | The starting value, as the table below says. |
| `label` | text | none | The name a person sees. |
| `group` | text | none | The heading it is listed under. |
| `unit` | string | none | The unit a person sees (`s`, `beats`, `rad/s`). |
| `min`, `max`, `step` | float | none | The range and step of a control. |
| `options` | array of text | `[]` | The choices of a `choice`. |

| `type` | `default` is | WGSL type | Notes |
|---|---|---|---|
| `float` | a number (an integer is fine) | `f32` | |
| `int` | an integer that fits `i32` | `i32` | Out of range: `<id>: out of range`. |
| `bool` | `true` or `false` | `u32` | 1 or 0. |
| `choice` | an integer, an index of `options` | `u32` | The value is the index. Out of range: `<id>: no option N` (a choice with no `options` gives this error too). |
| `color` | `"#rrggbb"`, sRGB | `vec4f` | Linear RGB, alpha 1. Another text: `"orange" is not a #rrggbb colour`. |

A `default` of the wrong kind is refused: `speed: "a" is not a Float value`. An unknown `type` is refused with the list (`unknown variant ..., expected one of float, int, bool, color, choice`).

The runtime reads `label`, `group`, `unit`, `min`, `max`, `step` and `options` as metadata for the tuning page and the lab; it does not enforce `min`, `max` or `step`, and it does not check `default` against them. A scene must not rely on a value staying in range.

### `[post]`

The defaults of the built-in post-processing, which takes `out` and makes the frame: bloom, tone mapping, chromatic aberration, vignette, grain and dithering. Every key is optional. The update may change these per frame through `post` (see "What WGSL gets").

| Key | Type | Default | Meaning |
|---|---|---|---|
| `exposure` | float, EV | `0.0` | Stops of light added before tone mapping. |
| `bloom` | table | see below | `{ intensity, threshold, radius }`; any key may be left out. |
| `tonemap` | word | `agx` | `agx`, `aces` or `none` (a plain sRGB encode). |
| `grain` | float | `0.04` | The strength of film grain. |
| `chromatic` | float | `0.002` | How far the colour channels shift at the frame's edge, as a share of its size. |
| `vignette` | float | `0.35` | How much the corners darken, 0 to 1. |
| `contrast` | float | `1.0` | The look on top of the tone mapping; 1 is neutral (AgX's "punchy" is 1.35 to 1.4). |
| `saturation` | float | `1.0` | The same, for colour; 1 is neutral. |

`bloom`: `intensity` is `0.6` (how much of the blurred light is added back), `threshold` is `1.0` (the HDR value above which light blooms, so `out` should hold values above 1 for what should glow), `radius` is `0.8` (0 to 1, how far the glow spreads).

The values are not checked for range, only for type.

### The tables `music.md` owns

Each is declared as below and explained in the section linked. Their errors are given there and by their own checks; the ones that matter when writing the file are in the last column.

| Table | Keys | What `music.md` says | Refused when |
|---|---|---|---|
| `[[input]]` | `id`, `label`, `range` (`[0, 1]`), `rest` | [Bindings](music.md#bindings) | an unknown key; `range` not `[lo, hi]` with `lo` below `hi`; more than 32 |
| `[[bind]]` | `target`, `source`, `how`, `when`, `ease`, `fallback` | [Bindings](music.md#bindings) | an unknown key; a target that is no input (`[[bind]] for a: no input of that name`); two binds for one input; an unknown source or a `when` key the `how` does not take |
| `[camera]` | `dutch` (0), `hold` (0), `pace` (`normal`), `dutch_in` (`all`) | [Shots and the director](music.md#shots-and-the-director) | `dutch` outside 0 to 1.5708 radians; `hold` below 0; the table without a `[[shot]]`; the removed `fill` |
| `[[shot]]` | `name`, `size` (required), `move` (`cut`), `opens`, `weight` (1), `longest`, `tiers`, `dutch`, `at_least_once` | the same | a `size` that is not `wide`, `medium`, `close`, `detail` or `top`; `weight` below 0 or all 0; `longest` of 0 or less; `dutch = true` on a wide or top shot; a `tiers` word that is not `quiet`, `normal` or `high` |
| `[[burst]]` | `name`, `size` (required) | the same | any other key (a burst takes no `weight` or `opens`); more than 4; no `[[shot]]` |
| `[[look]]` | `name`, `tags`, `weight` (1), `opens` (`true`), `tiers`, `at_least_once` | [Looks](music.md#looks) | an unknown key; `weight` below 0 or all 0; all `opens = false`; a bad `tiers` word |
| `[effects]` | `transitions`, `lens` | [Shared effects](music.md#shared-effects) | any other key; a lens effect under `transitions` or the reverse |

`pace` is `sections`, `calm`, `normal` or `busy`; `dutch_in` is `all` or `bursts`; `move` is `cut` or `glide`; `size` is `wide`, `medium`, `close`, `detail` or `top`; `tiers` takes `quiet`, `normal` and `high`. `transitions` takes `cut`, `fade`, `dissolve`, `cutin`, `flash`, `glitch` and `mosaic`; `lens` takes `flare`, `leak`, `blur`, `glitch` and `mosaic` (`blur` is accepted and not drawn yet). An `[effects]` name the runtime does not know (a newer scene on an older runtime) is left out with a warning, not refused. The old top-level `effects = [...]` list is refused with a message that shows the table to write.

### Typos and the TOML trap

Keys written after a table header belong to that table, not to the top level. This is plain TOML and a common slip:

```toml
[post]
tonemap = "aces"
covers = true       # this is post.covers, not the scene's covers
```

What happens then depends on the table. `[effects]`, `[[look]]`, `[[burst]]`, `[[input]]` and `[[bind]]` refuse any key they do not know (`unknown field covers, expected transitions or lens`), so the slip is found. Every other table and the top level ignore unknown keys, so the line above does nothing and the scene has no covers. Write the top-level keys first, before any `[table]`, and read the order of the file when something has no effect.

The same goes for `[[array]]` tables: a key after `[[shot]]` belongs to that shot until the next header.

## The update and the state

**The state.** `struct State` is declared in the file `[state] file` names. The runtime keeps one storage buffer of that struct for the scene. It starts as zeros, `update()` is the only code that writes it, every pass reads it, and it keeps its values from frame to frame. Its size is whatever WGSL's storage layout makes of the struct, at least 16 bytes. A scene without `[state]` gets a stand-in (`rv_unused`) it does not touch.

When a scene reloads (its files changed), the state is carried over only if the state file's text is unchanged and the struct has the same size; otherwise it starts again from zeros and `frame.first` is 1 for the first frame. A change of parameter defaults alone rebuilds nothing.

**The update.** `update()` is a function without arguments, in the file `[state] update` names. The runtime wraps it in a compute shader of one workgroup of one invocation and runs it once a frame, before the first pass, so it is single-threaded and every pass waits for it. It reads and writes `state` and `post`, and reads everything else a pass does but `covers`. It must be cheap: bound every loop, and see [Performance](music.md#performance) for its budget (1 ms on the reference phone). Put into the state what the passes need that is worked out once: positions, a camera, lists of what to draw, the counts that change on the beat.

The module of the update is the runtime's prelude, the state file, the update file and the entry point, in that order. It does not include the `common` files. If the update needs code that a pass also needs, put it in the state file, which every shader includes.

**The passes.** A pass's module is the prelude, the state file, the `common` files in the order listed, the declarations of the pass's inputs, the pass's file and the entry points. Several passes do not see each other's files; what they share goes in `common` or in the state file. Each pass draws one triangle that covers its target: the runtime supplies the vertex stage and calls `shade` for every pixel.

**The targets.** A pass draws into the buffer it names, or into `out`, which is always `rgba16float` (HDR) at the internal resolution. A buffer is `scale` times the internal resolution (`frame.resolution`, which is the output size times `[render] scale` times the factor the runtime has chosen for speed, and can change from frame to frame). A persistent buffer carries its last frame over such a change, stretched. A buffer starts empty (zeros). After each frame the persistent buffers swap their two textures, so `prev:<id>` in the next frame is what the buffer held at the end of this one.

**The post-processing** reads `out` after the last pass. The `post` struct is set to the `[post]` defaults before every `update()`; a field the update writes lasts for that one frame.

## What WGSL gets

Every module starts with the runtime's prelude, which declares the structs, the bindings and the `rv_*` functions below. The scene's own code must not redeclare them, so a scene may not use these names for its own globals: `frame`, `music`, `params`, `shot`, `look`, `input`, `state`, `post`, `spectrum`, `waveform`, `blue_noise`, `covers`, `linear_clamp`, `linear_repeat`, `nearest_clamp`, `asset_<id>`, the types `Frame`, `Music`, `Shot`, `Look`, `Post`, `Pixel`, `Params`, `Inputs`, `State`, the names of the buffers a pass reads, and anything starting with `rv_` or `Rv`. A local variable may shadow a global (a local `look` hides `look`), which is a trap: rename the local.

The shaders are checked with naga when the scene loads. A mistake in WGSL is reported as `file:line:column: message` in the scene's own file, and a line of the prelude as `<runtime>`; a reload that fails keeps the scene as it was.

### The bindings

| Name | What | `update()` | A pass |
|---|---|---|---|
| `frame` | `struct Frame`, uniform | read | read |
| `music` | `struct Music`, uniform: the raw fields | read | read |
| `params` | `struct Params`, uniform: the scene's `[[param]]`s | read | read |
| `input` | `struct Inputs`, uniform: the scene's `[[input]]`s | read | read |
| `shot` | `struct Shot`, uniform: the director's shot | read | read |
| `look` | `struct Look`, uniform: the scene's look | read | read |
| `state` | `struct State`, storage | read and write | read |
| `post` | `struct Post`, storage | read and write | not declared |
| `spectrum`, `waveform`, `blue_noise` | textures | yes | yes |
| `linear_clamp`, `linear_repeat`, `nearest_clamp` | samplers | yes | yes |
| `asset_<id>` | one texture per `[[asset]]` | yes | yes |
| `covers` | the album covers' atlas | not declared | yes |
| `<id>`, `prev_<id>` | the buffers a pass lists in `inputs` | no | yes, in the order listed |

### `frame`

| Field | Type | Meaning |
|---|---|---|
| `time` | f32 | Seconds since the scene's first frame, on the visualizer's clock (it stops while paused). |
| `dt` | f32 | Seconds since the last frame, at most 0.1; 1/60 on the first frame. Use `rv_dt()`, which caps it at 0.05. |
| `index` | u32 | The frame's number, from 0. |
| `first` | u32 | 1 on the scene's first frame, and after a reload that did not keep the state; else 0. Start the state from it. |
| `resolution` | vec2f | The internal resolution, in pixels. It changes with the runtime's quality steps: work sizes out from it each frame. |
| `aspect` | f32 | `resolution.x / resolution.y`. |
| `seed` | u32 | A new random number each frame (the same in tests that fix it). Not a per-run constant: two frames give two seeds. |
| `accent` | vec4f | The app's accent colour, linear. `rgb` is the colour and `a` is 1 once the app has told it, else 0 (a scene then falls back to a tint of its own). |
| `covers` | u32 | How many cells of the covers atlas are valid: 0 when there are none. Cell 0 counts even when the playing track has no cover. |
| `quality` | f32 | 1 at full quality, down to 0 in steps of 0.25 when the frame rate cannot be held at the lowest resolution. A scene takes fewer steps or samples at a lower one. See [Performance](music.md#performance). |

### `params` and `input`

`Params` has one member per `[[param]]`, in the order of the file, named by its `id`, of the WGSL type in the table of "`[[param]]`". A colour is a `vec4f` aligned to 16 bytes: read `params.tint.rgb`. A scene with no parameters gets a `rv_unused: u32` stand-in. `Inputs` has one `f32` per `[[input]]`, in the order of the file. These are the values the binds drive, and `rv_unused` stands in when there are none.

### `shot`

All zeros in a scene without `[[shot]]`. What each field means, and the rules the director follows, are in [Shots and the director](music.md#shots-and-the-director).

| Field | Type | |
|---|---|---|
| `index` | u32 | Which `[[shot]]`, from 0; the bursts follow the shots. |
| `cuts` | u32 | How many cuts so far. Frame the view again when it changes. |
| `size` | u32 | 0 wide, 1 medium, 2 close, 3 detail, 4 top. |
| `roll` | f32 | The dutch angle of this shot in radians, signed. |
| `seed` | f32 | 0 to 1, fixed for the shot. |
| `age` | f32 | Seconds since the shot began. |
| `length` | f32 | The planned seconds of the shot, -1 when unknown. |
| `fill` | u32 | 0, or the place in a burst, 1 to 4. |
| `hard` | u32 | 1 when the scene must jump to this shot, 0 when it may glide. |

### `look`

All zeros in a scene without `[[look]]`. See [Looks](music.md#looks).

| Field | Type | |
|---|---|---|
| `prev` | u32 | The look before, as its place among the `[[look]]`s from 0. (WGSL reserves `from`.) |
| `to` | u32 | The look now. |
| `mix` | f32 | 0 to 1 through the fade from `prev` to `to`, already eased. |
| `changes` | u32 | How many changes so far. |

### `post`

Only `update()` sees it. It starts every frame at the `[post]` defaults, and what is written to it for a frame is what the post-processing uses for that frame.

| Field | Type | Default |
|---|---|---|
| `exposure` | f32 | `[post] exposure` |
| `bloom_intensity`, `bloom_threshold`, `bloom_radius` | f32 | `[post] bloom` |
| `chromatic`, `vignette`, `grain` | f32 | `[post]` |
| `flash` | f32 | 0. Adds light to the whole picture, in proportion and as an offset. |
| `tint` | vec4f | `(1, 1, 1, 1)`. Multiplies the colour. |
| `contrast`, `saturation` | f32 | `[post]` |

The tone mapping is not a field: it is the one `[post] tonemap` set.

### `music`

`struct Music`, the raw fields, and the semantic signals after them. Every field, where its value comes from and what a scene should drive with it is in [Semantic signals](music.md#semantic-signals) and [Raw fields](music.md#raw-fields). A built-in scene does not read `music.` in its WGSL at all: it takes the music through `[[input]]` and `[[bind]]`, which fall back where the music cannot be measured and which the lab can edit (a test in `src/builtin.rs` searches the built-in scenes' WGSL for it). A scene of the user's may read any field.

### Textures and samplers

- `spectrum`: 256 columns, 30 Hz to 16 kHz on a log scale, by 128 rows, one per frame with the newest first. Read it with `rv_spectrum`.
- `waveform`: 1024 stereo samples, the latest, oldest to newest. Read it with `rv_waveform`.
- `blue_noise`: 64 by 64, four channels in 0 to 1, tiling. Read it with `rv_blue_noise`.
- `covers`: for a scene with `covers = true`, an atlas of 1024 by 1024 pixels (sRGB, with mips) of 8 by 8 cells of 128 pixels, row-major from the top left, each cover scaled to fill its cell (the middle square of it). Cell 0 is the playing track's cover (clear if it has none); the others are random covers from the library, which are filled in the background a little after the scene loads. A scene without the key sees one clear texel and `frame.covers` of 0. Passes only.
- `asset_<id>`: the image, `texture_2d<f32>`, with all its mips.
- `<id>` and `prev_<id>`: a pass's inputs, `texture_2d<f32>` in group 1, at the binding of their place in `inputs`. A buffer of `rgba32float` cannot be filtered: read it with `textureLoad`.
- The samplers: `linear_clamp`, `linear_repeat`, `nearest_clamp`.

All of them are `texture_2d<f32>`, so sample with `textureSampleLevel` (or `textureLoad`), as the shaders of the built-in scenes do.

### The entry points

The update defines `fn update()`. A pass defines

```wgsl
fn shade(px: Pixel) -> vec4f
```

and returns linear colour. In `out` and in `rgba16float` buffers, values above 1 are kept and bloom from the threshold up. In an `rgba8unorm` buffer the result is clamped to 0 to 1.

`struct Pixel`:

| Field | Type | Meaning |
|---|---|---|
| `uv` | vec2f | 0 to 1 over the target, from its top left. |
| `p` | vec2f | Centred and square: the target's height spans -0.5 to 0.5 with y up, the width spans `aspect` times that. |
| `coord` | vec2f | The pixel's position in the target in pixels, from the top left (at the pixel's centre, so `x.5`): use it with `textureLoad`. |
| `size` | vec2f | The target's size in pixels: `floor(frame.resolution * scale)`, where `scale` is the target buffer's, 1 for `out`. |

### The library

`stdlib.wgsl` provides stateless functions, all named `rv_*`, and two constants, `RV_PI` and `RV_TAU`. They are in every module. [Helpers for these patterns](music.md#helpers-for-these-patterns) shows how the music patterns use them.

| Function | Meaning |
|---|---|
| `rv_hash(v: u32) -> u32`, `rv_hash2(v: vec2u) -> u32` | PCG hash of one word, of two. |
| `rv_unit(v: u32) -> f32` | 0 to 1 from a word. |
| `rv_rand2(p: vec2f) -> f32` | 0 to 1 from a point; the points of one lattice cell (the same `floor`) give one number. |
| `rv_rand1(x: f32) -> f32` | 0 to 1 from any float, by its bits. There is no `rv_rand(seed)`. |
| `rv_value_noise(p: vec2f) -> f32` | Value noise, 0 to 1. |
| `rv_gradient(i: vec2f) -> vec2f` | The unit gradient at a lattice point. |
| `rv_noise(p: vec2f) -> f32` | Gradient noise, about -0.7 to 0.7. |
| `rv_fbm(p: vec2f, octaves: i32) -> f32` | Sum of octaves of `rv_noise`, each half as strong. |
| `rv_sd_circle(p, r)`, `rv_sd_box(p, half_size)`, `rv_sd_round_box(p, half_size, r)` | Signed distances (negative inside) of a circle and a box (`vec2f` arguments). |
| `rv_sd_segment(p, a, b)` | The distance to a segment, always 0 or more. |
| `rv_band(x, a, b, aa) -> f32` | 1 inside `a` to `b`, with edges `aa` wide. |
| `rv_rotate(p: vec2f, a: f32) -> vec2f` | `p` turned by `a` radians. |
| `rv_srgb_to_linear(c)`, `rv_linear_to_srgb(c)`, `rv_luma(c)` | Colour (`vec3f`): sRGB to linear and back, and the luma. |
| `rv_linear_to_oklab(c)`, `rv_oklab_to_linear(c)` | Linear RGB and OKLab. |
| `rv_hue_rotate(c: vec3f, a: f32) -> vec3f` | Turns the hue by `a` radians, keeping lightness (in OKLab). |
| `rv_dt() -> f32` | `min(frame.dt, 0.05)`: one upper limit for the time step in every scene. |
| `rv_approach(x, goal, dt, tau) -> f32` | `x` moving towards `goal` with time constant `tau`, the same at any frame rate. |
| `rv_env(prev, input, dt, tau) -> f32` | An envelope: `prev` fading with time constant `tau`, never below `input`. |
| `rv_pulse(phase, sharpness) -> f32` | A spike at every whole number of `phase`, falling off with `sharpness`. |
| `rv_beat_pulse(clock, sharpness) -> f32` | `rv_pulse` with a beat clock as the phase. |
| `rv_decay(t, half_life) -> f32` | What is left of 1 after `t` seconds, halving every `half_life`. |
| `rv_label_hash(label: f32, k: u32) -> f32` | 0 to 1, the same for the same section label and different for each `k`. |
| `rv_spectrum(x: f32, age: f32) -> f32` | The spectrum at `x` (0 to 1) and `age` rows back (0 is now). |
| `rv_waveform(x: f32) -> vec2f` | The stereo sample at `x` (0 oldest, 1 newest). |
| `rv_blue_noise(coord: vec2f, animated: bool) -> vec4f` | Blue noise at a pixel; a different pattern each frame if animated. |

### Rules

- **A built-in scene never writes `music.`** (see "`music`"), and never reads a raw field that has a semantic replacement.
- **`from` and `target` are reserved words in WGSL.** Do not use them as names, and use another word for what they would name (`prev` and `to`, as `struct Look` does).
- **`a && b || c` needs parentheses** in WGSL: `(a && b) || c`.
- **`frame.seed` changes every frame.** A random number drawn from it alone (`rv_unit(frame.seed * K + n)`) is a different number every frame, so it is no way to decide something once; draw it at the moment of the decision (a cut, a bar, `frame.first`) and keep the result in the state. Mix a counter or the bar into the draw when two decisions are made in one frame.
- **Only `update()` writes `state` and `post`.** A pass that writes them is refused by the shader check.
- **The update is one thread.** Bound its loops and keep its work small.
- **Do not cache a size in pixels.** Take it from `frame.resolution` or `px.size` every frame.
- **Declare no parameter that nothing reads**, and no input that no bind drives and no shader reads.

## An example

A scene that draws a ring that turns at the speed of a parameter and swells on the music's hardest hits. It uses one parameter of each of two types, one input with its bind, a state and an update, and one pass. It is five files in a folder, and it is a complete scene: the parser accepts it and it draws.

`scene.toml`:

```toml
format = 1
name = "Pulse Disc"
license = "MPL-2.0"
description = { en = "A ring of light that turns with the music's pace and swells on its hardest hits." }

[state]
file = "state.wgsl"
update = "update.wgsl"

[[param]]
id = "spin"
type = "float"
label = { en = "Spin" }
group = { en = "Beat" }
unit = "rad/s"
default = 0.6
min = 0.0
max = 3.0
step = 0.05

[[param]]
id = "tint"
type = "color"
label = { en = "Colour" }
group = { en = "Look" }
default = "#ff8040"

[[input]]
id = "surge"

[[bind]]
target = "surge"
source = "impacts"
how = "trigger"
when = { release = 0.5 }
fallback = "hits"

[[pass]]
shader = "disc.wgsl"
target = "out"

[post]
bloom = { intensity = 0.5 }
```

`state.wgsl`:

```wgsl
struct State {
  angle: f32,
  radius: f32,
}
```

`update.wgsl`:

```wgsl
fn update() {
  if (frame.first == 1u) {
    state.radius = 0.2;
  }
  let dt = rv_dt();
  state.angle += dt * params.spin;
  state.radius = rv_approach(state.radius, 0.2 + 0.1 * input.surge, dt, 0.08);
}
```

`disc.wgsl`:

```wgsl
fn shade(px: Pixel) -> vec4f {
  let p = rv_rotate(px.p, state.angle);
  // A ring with a gap that turns, so the rotation shows.
  let ring = abs(rv_sd_circle(p, state.radius)) - 0.02;
  let gap = rv_band(atan2(p.y, p.x), -0.4, 0.4, 0.05);
  let body = (1.0 - smoothstep(0.0, 0.004, ring)) * (1.0 - gap);
  let light = params.tint.rgb * 2.0 * body;
  return vec4f(light, 1.0);
}
```

The built-in scenes are the fuller examples. `visualizer/scenes/armillary/` is the one to read first: one pass, with shots, bursts, looks and effects, `common` and a state and an update. `torii-sea/` is the same shape with four looks. `growth/` has two buffers, three passes and an asset. `neon-street/` has four buffers, one of them persistent, and five passes. `crt-wall/` also reads the covers (`covers = true`), and `emoji-tunnel/` has two assets.

To see a scene while writing it: add its folder in Settings > Theme, lock it from the cover wall's context menu so the switching leaves it alone, and edit; the scene reloads on every save and shows a problem in the visualizer's corner until the next save fixes it. `cargo run -p visualizer --example snapshot -- <dir> [scene folder]` renders a scene to images (see `README.md`).

## Versioning

`format = 1` is the format this document describes. The runtime reads every format from 1 up to the one it knows (`FORMAT` in `src/scene/manifest.rs`, now 1). A scene that says a newer one is refused with `format N is not supported (this runtime reads up to M)`, so a later format may change what a key means and an older runtime will not guess. So does a format below 1.

Inside format 1 the file may grow keys and tables. A key a runtime does not know is ignored by every table that ignores unknown keys (see "Typos and the TOML trap"), so a scene written with a newer key still loads on an older runtime and simply does without it; an unknown effect name is left out with a warning. A key that was removed is not ignored: it is refused by name, with a message that says what to write instead. Two are removed so far: `[camera] fill` (declare `[[burst]]` shots) and a top-level `effects = [...]` (declare an `[effects]` table).

How a mistake is reported:

- A file that is not valid TOML, a missing required key, a word that is not one of a fixed list, a value of the wrong type, and an unknown key of a table that refuses them: a TOML error that names the line and the column and shows the line.
- A value the parser can read but the format does not allow (a range, a name, a path, a limit, a missing target): the messages in the tables above.
- Every one is reported with the file name in front, as `scene.toml: ...`, and refuses the scene: the visualizer shows it in its corner and, for a scene that was already running, keeps the old one.
- A mistake in WGSL is found when the scene is built, not when `scene.toml` is read: `file:line:column: message`.

## What lives elsewhere

- [`music.md`](music.md): what each field of `music` means and which to read; bindings and their sources, hows and fallbacks; the camera director and shots; looks; the shared transitions and lens effects; the parameters for tuning; the performance budget and the rules for authors.
- `README.md`: how the folders are scanned and switched between, the frame, the beat tracker and the chart, adaptive quality, benchmarking and how frames reach Flutter.
- `src/scene/manifest.rs`, `src/scene/wgsl.rs`, `src/scene/stdlib.wgsl`: the source of this document.
