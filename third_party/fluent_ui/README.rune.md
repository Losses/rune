# Rune Fluent UI source dependency

Runtime sources/assets copied from https://github.com/Losses/fluent_ui at commit `fa35f8cae87d2c0a9989279fe99fdee32d601e84` (package 4.9.1). The upstream MIT license is retained in LICENSE. Example apps, tooling and upstream tests are omitted.

Rune uses this source dependency so fixes apply reproducibly to local and CI builds on every platform, without modifying the shared Pub cache or requiring an unpublished remote commit.

Local patches:

- `styles/acrylic.dart`: apply tint and luminosity blends inside the clipped backdrop layer. Keep the blur and noise layers enabled.
- `flyouts/content.dart`: isolate each menu row so hover changes do not repaint unchanged rows or the acrylic background.
- `flyouts/flyout.dart`: normalize hover positions into Navigator coordinates; dismiss the owning flyout when geometry changes without popping a newer route.
- `flyouts/menu.dart`: normalize submenu pointer geometry into Navigator coordinates.
- `surfaces/tooltip.dart`: normalize cursor placement into root-overlay coordinates and invalidate stale geometry, including keyboard/safe-area changes.
- `form/selection_controls.dart`: correct default text-menu selection and secondary-click anchors, including multiline selection, for scaled and translated root overlays.
- `form/text_box.dart`: scoped Android/iOS EditableText size compatibility for Flutter 3.47 toolbar restoration; restore application metrics for context-menu builders; `TextBox.adaptiveContextMenuBuilder` recomputes scaled overlay anchors for modern menus. Recheck against SDK internals on upgrades.
- `utils/hover_button.dart`, `inputs/slider.dart`: keep the Material slider's OverlayPortal anchor out of forced semantics merging, so its traversal children remain reachable. HoverButton retains merging by default. Use the existing Fluent tooltip for a slider's hover label instead of calling private Material slider state; the tooltip uses overlay coordinates, stays upright for vertical sliders, is hidden during dragging, and remains available for disabled sliders. Material's own drag value indicator is unchanged. Slider edge-label correction uses overlay coordinates.
- `form/popup_metrics.dart`, combo box, picker and auto-suggest controls: invalidate cached popup geometry and capture the exact route/entry to remove.
- `form/auto_suggest_box.dart`: while an input method is composing, pass candidate-navigation keys to the platform without allowing ancestor shortcuts to consume them; retain suggestion navigation when composition ends.

Regression coverage lives in the application's `test/widgets` and `test/providers` directories. When updating upstream, review every patch and run the complete scaling suite. Once equivalent fixes exist in the upstream fork, this directory may be replaced with a pinned Git dependency and the corresponding Nix Git hash. Do not patch a shared Pub cache: CI must build the same sources.
