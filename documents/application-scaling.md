# Application scaling

Use this guide when adding UI, changing responsive layouts, or integrating native input and rendering with Rune's interface-scale setting.

## Change the interface scale

The Theme settings page offers 75%, 100%, 125%, 150%, 175% and 200%. The value multiplies the operating system's scale. At 150% system scale and 125% app scale, one app logical pixel occupies 1.875 physical pixels. Ctrl+0 on Windows and Linux and Cmd+0 on macOS restore 100%.

Use `AppScaleProvider.setPercentage` to change the setting:

```dart
await context.read<AppScaleProvider>().setPercentage(125);
```

The provider validates the percentage, writes the profile's `interface_scale` setting and then notifies listeners. Handle a failed write at the caller. The displayed percentage stays at its previous value. Read `percentage` for the selector and `scaleFactor` for root layout calculations. Supported values are defined by `AppScaleProvider.supportedPercentages` in `lib/providers/app_scale.dart`.

## Add or resize a widget

Use Flutter logical pixels for `Padding`, text, layout constraints and local Canvas drawing. `AppViewport` in `lib/widgets/app_viewport.dart` scales the Navigator and its overlays. Read layout dimensions from `LayoutBuilder` or the widget's `MediaQuery`.

Inside `AppViewport`, with app scale factor `s`:

```text
UI viewport size = unscaled viewport size / s
MediaQuery.devicePixelRatio = View.devicePixelRatio * s
Physical pixels = UI logical pixels * MediaQuery.devicePixelRatio
```

When changing `AppViewport`, update size, padding, insets, display-feature rectangles and corner radii using the same factor. Keep the root widget structure stable at 100% and other percentages. Preserve the system text scaler. Check these changes with `test/providers/app_viewport_test.dart`.

Use `ScreenSizeProvider.screenSize` for responsive breakpoints and `rawWindowSize` for native window-size persistence. Both are defined in `lib/providers/responsive_providers.dart`.

When a responsive layout moves an active editor between subtrees:

1. Store its controller and focus node in the owning State.
2. Give the moving editor a State-owned `GlobalKey` so Flutter reparents its existing state.
3. Dispose controllers created by that State. Leave caller-owned controllers to the caller.
4. Test the layout change while the field has focus and an active composing range.

See `test/widgets/search_breakpoint_input_test.dart` and the editor tests in `test/dialogs/`.

## Convert pointer and overlay positions

Convert a global pointer position into the receiving render box's local coordinates before using it in a painter:

```dart
final box = context.findRenderObject()! as RenderBox;
final localPosition = box.globalToLocal(event.position);
```

For a stationary-pointer effect, retain the global position and convert it again after layout or scale changes. Use local coordinates for the shader's mouse and resolution uniforms, and `FlutterFragCoord()` for the fragment position. The implementations are in `lib/widgets/ax_reveal/`, `lib/widgets/cover_wall_background/gradient_container.dart` and `lib/shaders/gradient.frag`.

When positioning an overlay, convert both the target and the pointer into the overlay render box's coordinates:

```dart
final targetBounds = MatrixUtils.transformRect(
  targetBox.getTransformTo(overlayBox),
  Offset.zero & targetBox.size,
);
final pointerInOverlay = overlayBox.globalToLocal(event.position);
```

`targetBounds` includes the transformed origin and size. Recompute cached bounds when scale, window size or insets change. If closing a popup instead, remove the specific route or entry that owns it. Test translated targets as well as scaled targets using `test/widgets/fluent_overlay_geometry_test.dart` and `test/widgets/fluent_popup_metrics_test.dart`.

## Add text input or a selection menu

Use Fluent `TextBox` or `AutoSuggestBox` for application inputs. For an adaptive selection menu, set `contextMenuBuilder: TextBox.adaptiveContextMenuBuilder`. Custom menus must derive anchor positions from `RenderEditable` and convert them into the root overlay's coordinates.

When modifying suggestion-key handling, check the controller's composing range first. While that range is valid and nonempty, return `KeyEventResult.skipRemainingHandlers` for Escape, Enter, numpad Enter, Space, arrow keys, Page Up and Page Down. This stops ancestor shortcuts and lets the platform input method receive those keys. Once composition ends, resume suggestion navigation and dismissal. The handler is in `third_party/fluent_ui/lib/src/controls/form/auto_suggest_box.dart`.

Run `test/widgets/auto_suggest_ime_keys_test.dart` for keyboard changes and `test/providers/desktop_selection_scale_test.dart` and `test/providers/mobile_selection_scale_test.dart` for selection-menu changes. On Android and iOS, retain the vendored TextBox's raw-view-size override around EditableText and its application MediaQuery around the menu builder. Test toolbar restoration with the keyboard visible.

When changing Fluent Slider or HoverButton, retain the Slider's Material semantics instead of merging its descendants through HoverButton. Check `test/widgets/fluent_slider_semantics_test.dart` and operate the slider with the platform screen reader.

For Windows framework changes, use the SDK preparation and upgrade steps in [Windows Flutter SDK](windows-flutter-package.md#framework-compatibility).

## Allocate images and visualizer textures

Compute a raster target from the local logical size and `MediaQuery.devicePixelRatioOf(context)`. Recompute it when either value changes.

For an asynchronous image load, associate each request with a generation number. Discard and dispose a completed image if a newer request has replaced it or the widget has been disposed. Dispose images owned by the widget when replacing them. Use the cache owner's release mechanism for shared images. See `lib/utils/load_and_resize_image.dart` and `test/widgets/cover_wall_scale_test.dart`.

For visualizer textures, call `visualizerRenderSize(localSize, effectiveDpr)` from `lib/widgets/visualizer/visualizer_render_size.dart`. It applies the 1920×1080 limit, 64-pixel width alignment and even-height requirement. Pass only finite, positive dimensions. Use the existing 300 ms resize debounce in `VisualizerSurface`.

On Windows, the Flutter bridge defaults to readback. To test shared DXGI, set `RUNE_WINDOWS_DXGI_SHARED=1` in the test process's environment and restart the visualizer. `RUNE_VISUALIZER_READBACK=1` forces readback and takes precedence. Check displayed frames with the [visualizer procedure](#visualizer-procedure) whenever changing the transport.

## Integrate native windows and file drops

Use `View.of(context).devicePixelRatio` when converting native physical pixels into unscaled Flutter view coordinates. Use `nativeWindowLogicalToLocal(context, length)` from `lib/utils/native_window_metrics.dart` when reserving Flutter layout space for an OS-sized decoration, such as macOS window buttons.

Use `AppDropTarget` from `lib/widgets/router/app_drop_target.dart` for desktop file drops. It supplies the plugin with the raw view DPR and restores the app's MediaQuery for descendants. The Windows plugin converts physical input using that DPR. The macOS plugin supplies window points. Let the render tree convert the resulting points into the drop target's local coordinates.

When changing a native view's placement inside its window, verify the native event's origin against that view's origin. Test file delivery at opposite window edges using the steps below.

## Regression testing

### Automated regression

Resolve packages with the selected Flutter SDK, then run:

```sh
flutter pub get
flutter analyze --no-pub
flutter test --no-pub
```

For a focused run during development:

```sh
flutter test --no-pub test/providers test/settings test/widgets test/dialogs
```

### Native acceptance cases

Build and cold-launch the revision under test. Use a separate settings profile and test library. For a silent run, prepare `volume_level` as double `0.0`, `disable_branding_animation` as `true`, and `branding_sfx` as `"mute"` before launch. Profile-qualified keys use `ci.not.rune#scale-validation:<key>`. Confirm the app is muted and stopped before a drop or playback test.

A debug launch on macOS is:

```sh
flutter run --debug -d macos --dart-entrypoint-args=--profile --dart-entrypoint-args=scale-validation
```

Use `-d windows` or `-d linux` on those platforms. A profile separates settings keys. Use a separate test account or macOS app identifier when filesystem isolation is needed. Build setup is in [Compiling Rune](compiling.md).

#### Scale setting and persistence

Select each of the six percentages in Theme at a fixed window size. Check that text and controls align with their hit targets and that the native window dimensions stay unchanged. Resize into compact layouts and confirm that the scale setting remains reachable.

Focus a field and enter text. Select 125%, then use Ctrl+0 on Windows or Linux, or Cmd+0 on macOS. Check that the scale returns to 100% while the text and focus remain intact. Select 125% again and restart the same profile to check persistence.

#### Popups and text input

Open a flyout, submenu, tooltip, selection menu and Mix Studio near a window edge. Change scale and resize across a responsive breakpoint. Check that each popup repositions or closes its own entry and that the editor retains its text, focus and composing range.

Test native IME input at 75%, 100%, 150% and 200%. Select the scale before starting each composition. Navigate candidates and commit a selection, then compose again and cancel with Escape. Check that the candidate window does not obscure the composing text, committing inserts the selected text, and canceling preserves previously committed text and focus. Finish each composition before changing scale for the next sample.

#### File drops and pointer effects

Drag a silent fixture from Explorer or Finder onto targets near different window edges at 75%, 125% and 200%. Confirm in the app or its log that Rune loads the dropped file exactly once. Do not issue a separate playback command to verify delivery.

Hover a control with a lighting effect, leave the pointer stationary and change scale. Check that the light remains aligned with the pointer. Drag cover art while resizing the window, then cancel the drag. Check that the drag image tracks the pointer and returns to its original position on cancellation.

#### Native window controls and accessibility

On macOS, test the native window buttons at every percentage. Resize across compact layouts, restore the window and click the buttons. Check that their hit areas retain the OS size and do not overlap Flutter controls. Buttons should be hidden in `band` and `dock` layouts. They should also be hidden in `zune` and `phone` layouts when the cover-art wall is shown. In other layouts, they should be visible.

Navigate to sliders with the platform screen reader and adjust their values while changing scale and window size. Check that the reader announces the name and current value, that focus stays on the selected control, and that increment and decrement actions work. Check the native logs for invalid AXTree updates.

#### System DPI changes

Move the window between displays with different OS scales at app scales of 100%, 125% and 200%. Keep an input field, a popup and the visualizer active. Check that view metrics and texture dimensions update for the destination display, that hit targets align with controls, and that the candidate window stays next to the composing text.

For debug metric inspection, use the app's VM service URL:

```sh
python3 scripts/scale_runtime_probe.py 'http://127.0.0.1:PORT/TOKEN=/'
```

The probe checks viewport dimensions and effective DPR. Its `--set-percent` option sets a test precondition. Exercise the Theme selector separately with native input.

### Visualizer procedure

1. Open the cover-wall visualizer in a layout that contains `VisualizerSurface`. Start a silent fixture and select a scene with time-based animation.
2. At each percentage, wait for the 300 ms resize debounce. Compare the requested size from `visualizerRenderSize` with the renderer's reported width and height. Check `running`, `lastError`, adapter and transport.
3. Resize the native window and repeat. Verify that the displayed scene follows the new surface size without stretching, blank frames or a retained old texture.
4. Capture two frames several seconds apart while playback advances. Inspect the visualizer region and compare decoded RGB pixels. Label captures as Flutter texture rasters or compositor screenshots.
5. Stop playback, restore the test settings and remove any transport override from the test launch environment.

Record the source revision, build mode, SDK version, OS, GPU, driver version, display scales and input method with each test result. Attach logs and test-window captures to the change or CI run. Restore the input source and moved test windows, then close the test app and debug connections.
