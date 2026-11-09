import 'dart:async';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'app_drop_target.dart';
import 'package:fast_file_picker/fast_file_picker.dart';

import '../../main.dart';

import '../../utils/kept_clear.dart';
import '../../utils/query_list.dart';
import '../../utils/playing_item.dart';
import '../../utils/filter_audio_files.dart';
import '../../utils/is_cover_art_wall_layout.dart';
import '../../utils/show_visualizer_import.dart';
import '../../utils/api/operate_playback_with_mix_query.dart';
import '../../screens/bsod/bsod.dart';
import '../../widgets/navigation_bar/flip_animation.dart';
import '../../widgets/navigation_bar/navigation_bar.dart';
import '../../widgets/navigation_bar/navigation_back_button.dart';
import '../../widgets/banding_animation/branding_animation.dart';
import '../../widgets/playback_controller/cover_art_disk.dart';
import '../../widgets/playback_controller/playback_controller.dart';
import '../../widgets/status_bar_style.dart';

import '../../providers/crash.dart';
import '../../providers/router_path.dart';
import '../../providers/library_open_state.dart';
import '../../providers/responsive_providers.dart';
import '../../providers/visualizer_scenes.dart';
import '../visualizer/visualizer_dark_theme.dart';
import '../visualizer/lab/lab_mode_builder.dart';
import '../../providers/visualizer_lab.dart';

import '../../bindings/bindings.dart';

import 'rune_stack.dart';
import 'scale_fade_container.dart';

class RuneRouterFrameImplementation extends StatefulWidget {
  const RuneRouterFrameImplementation({super.key, required this.child});

  final Widget child;

  @override
  State<RuneRouterFrameImplementation> createState() =>
      _RuneRouterFrameImplementationState();
}

class _RuneRouterFrameImplementationState
    extends State<RuneRouterFrameImplementation> {
  final _playbackControllerKey = GlobalKey();

  /// Whether the one-shot startup branding animation is still shown.
  ///
  /// The branding animation only plays once per app launch. Keeping it mounted
  /// after it finished would leave an invisible layer in the tree that keeps
  /// scheduling frames (its lens flare used to tick every 16ms), so it is
  /// removed as soon as it completes.
  late bool _showBrandingAnimation;

  // Imports of scene files the system asked Rune to open, to tell about.
  StreamSubscription<VisualizerImport>? _sceneImports;

  @override
  void initState() {
    super.initState();
    _showBrandingAnimation = !disableBrandingAnimation;
    _sceneImports = context.read<VisualizerScenesProvider>().imports.listen((
      result,
    ) {
      if (mounted) showVisualizerImport(context, result);
    });
  }

  @override
  void dispose() {
    _sceneImports?.cancel();
    super.dispose();
  }

  // Scene files dropped on the window are imported; the rest is played.
  Future<void> _importScenes(List<String> paths) async {
    final provider = context.read<VisualizerScenesProvider>();
    for (final path in paths) {
      final result = await provider.importScene(path);
      if (!mounted) return;
      showVisualizerImport(context, result);
    }
  }

  void _handleBrandingAnimationCompleted() {
    if (!mounted || !_showBrandingAnimation) {
      return;
    }
    setState(() {
      _showBrandingAnimation = false;
    });
  }

  @override
  Widget build(BuildContext context) {
    if ($router.path == '/') {
      return widget.child;
    }

    final library = Provider.of<LibraryOpenStateProvider>(context);
    final r = Provider.of<ResponsiveProvider>(context);
    final crash = Provider.of<CrashProvider>(context);

    final isCar = r.smallerOrEqualTo(DeviceType.car, false);
    final isZune = r.smallerOrEqualTo(DeviceType.zune, false);
    final diskOnRight = r.smallerOrEqualTo(DeviceType.car, false);

    final showDisk = isZune || isCar;

    if (crash.report != null) {
      // The report is on a dark background in both themes.
      return StatusBarStyle(
        brightness: Brightness.dark,
        child: Bsod(report: crash.report!),
      );
    }

    if (!library.hasOpenLibrary) {
      return Container();
    }

    final mainContent = FocusTraversalOrder(
      order: const NumericFocusOrder(2),
      child: widget.child,
    );

    final path = Provider.of<RouterPathProvider>(context).path;
    final isSpecialLayout = isCoverArtWallLayout(path);
    final labMode = watchVisualizerLabMode(context);
    final keptClear = context.keptClear;

    // Create the PlaybackController with a key once
    final playbackController = KeyedSubtree(
      key: _playbackControllerKey,
      // Over the visualizer the cover wall is dark, so is the controller.
      child: VisualizerDarkTheme(
        enabled: path == '/cover_wall',
        child: const FocusTraversalOrder(
          order: NumericFocusOrder(3),
          child: PlaybackController(),
        ),
      ),
    );

    return AppDropTarget(
      onDragDone: (detail) {
        bool isScene(String path) => path.toLowerCase().endsWith('.runescene');
        final scenes = detail.files.map((x) => x.path).where(isScene).toList();
        if (scenes.isNotEmpty) _importScenes(scenes);
        final files = detail.files
            .where((xfile) => !isScene(xfile.path))
            .map((xfile) => FastFilePickerPath.fromPath(xfile.name, xfile.path))
            .toList();
        if (files.isEmpty) return;

        safeOperatePlaybackWithMixQuery(
          context: context,
          queries: QueryList(),
          playbackMode: 99,
          hintPosition: -1,
          initialPlaybackId: -1,
          instantlyPlay: true,
          operateMode: PlaylistOperateMode.replace,
          fallbackPlayingItems: filterAudioFiles(
            files,
          ).map(PlayingItem.independentFile).toList(),
        );
      },
      child: Stack(
        children: [
          if (_showBrandingAnimation)
            BrandingAnimation(onCompleted: _handleBrandingAnimationCompleted),
          ScaleFadeContainer(
            delay: disableBrandingAnimation
                ? const Duration(milliseconds: 0)
                : const Duration(milliseconds: 4350),
            duration: disableBrandingAnimation
                ? const Duration(milliseconds: 200)
                : const Duration(milliseconds: 500),
            child: FlipAnimationContext(
              child: FocusTraversalGroup(
                policy: OrderedTraversalPolicy(),
                child: RuneStack(
                  alignment: diskOnRight
                      ? Alignment.centerRight
                      : Alignment.bottomCenter,
                  children: [
                    if (isSpecialLayout && !showDisk) mainContent,
                    // Always keep the PlaybackController in the widget tree
                    // but make it visible only when needed
                    if (!showDisk)
                      Visibility(
                        visible: true, // Always in the tree
                        maintainState: true, // Keep the state
                        // In the lab mode the controller moves to the top of
                        // the window. It is the same widget, so it plays on.
                        // At the bottom it stays above Android's navigation
                        // bar, and runs under the home indicator on iOS.
                        child: AnimatedAlign(
                          alignment: labMode
                              ? Alignment.topCenter
                              : Alignment.bottomCenter,
                          duration: labModeDuration,
                          curve: labModeCurve,
                          child: AnimatedPadding(
                            padding: EdgeInsets.only(
                              top: labMode ? keptClear.top : 0,
                              bottom: labMode ? 0 : keptClear.bottom,
                            ),
                            duration: labModeDuration,
                            curve: labModeCurve,
                            child: playbackController,
                          ),
                        ),
                      ),
                    FocusTraversalOrder(
                      order: const NumericFocusOrder(1),
                      child: DeviceTypeBuilder(
                        deviceType: const [
                          DeviceType.band,
                          DeviceType.dock,
                          DeviceType.tv,
                        ],
                        builder: (context, activeBreakpoint) {
                          final isSmallView =
                              activeBreakpoint == DeviceType.band ||
                              activeBreakpoint == DeviceType.dock;

                          if (!isSmallView) {
                            // The bar slides out through the top of the
                            // window in the lab mode.
                            return LabModeBuilder(
                              builder: (context, value, on, child) =>
                                  IgnorePointer(
                                    ignoring: on,
                                    child: Transform.translate(
                                      offset: Offset(
                                        0,
                                        -value * (240 + keptClear.top),
                                      ),
                                      child: child,
                                    ),
                                  ),
                              child: NavigationBar(path: path),
                            );
                          }

                          return Positioned(
                            top: -12 + keptClear.top,
                            left: -12 + keptClear.left,
                            child: const NavigationBackButton(),
                          );
                        },
                      ),
                    ),
                    if (!(isSpecialLayout && !showDisk)) mainContent,
                    if (showDisk)
                      const BlockHitTestStack(
                        child: FocusTraversalOrder(
                          order: NumericFocusOrder(4),
                          child: CoverArtDisk(),
                        ),
                      ),
                  ],
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}
