# Linux rendering

Rune selects Skia through `fl_dart_project_set_enable_impeller` in `linux/my_application.cc`. The native visualizer keeps its Vulkan renderer and shared-texture transport. Leave Mesa threading and cache settings at their defaults.

Before initializing the application, `lib/main.dart` awaits the Fluent UI fork's `AcrylicShaderWarmUp` on Linux. It compiles acrylic drawing operations into a temporary 512 by 512 image using the native pixel ratio and the supported interface scales. A warm-up failure is logged and startup continues. Widgets do not need individual warm-up calls.

When changing the renderer, Flutter version, or acrylic painting, test a release build with both a new cache directory and an existing cache. Use an isolated settings profile at zero volume. Measure first opening, repeated opening after idle, and pointer movement across the home and visualizer context menus. Record Flutter build and raster durations separately; they do not measure compositor presentation latency.

Check light and dark themes, all supported interface scales, rounded clipping, backdrop blur, pointer lighting, and native visualizer playback. Confirm the visualizer's transport and frame delivery in its diagnostics. Keep shader inputs and animation time fixed when comparing renderer screenshots. Run the Fluent UI fork's acrylic and flyout tests, then Rune's `lib/test` suite. Store traces and screenshots outside the source checkout.
