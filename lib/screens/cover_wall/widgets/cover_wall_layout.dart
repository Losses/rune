import 'package:flutter/services.dart';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../utils/l10n.dart';
import '../../../utils/color_brightness.dart';
import '../../../utils/visualizer_supported.dart';
import '../../../utils/router/router_aware_flyout_controller.dart';
import '../../../widgets/keep_clear.dart';
import '../../../widgets/context_menu_wrapper.dart';
import '../../../widgets/visualizer/visualizer_controller.dart';
import '../../../widgets/visualizer/visualizer_surface.dart';
import '../../../widgets/visualizer/visualizer_problem.dart';
import '../../../widgets/visualizer/lab/lab_mode_builder.dart';
import '../../../widgets/visualizer/lab/lab_overlay.dart';
import '../../../widgets/cover_wall_background/gradient_container.dart';
import '../../../widgets/cover_wall_background/cover_wall_background.dart';
import '../../../widgets/cover_wall_background/utils/calculate_cover_wall_size.dart';
import '../../../widgets/playback_controller/constants/playback_controller_height.dart';
import '../../../providers/responsive_providers.dart';
import '../../../providers/visualizer_lab.dart';
import '../../../providers/visualizer_scenes.dart';

import 'back_button.dart';
import 'playing_track.dart';

class CoverWallLayout extends StatefulWidget {
  const CoverWallLayout({super.key});

  @override
  CoverWallLayoutState createState() => CoverWallLayoutState();
}

const gradientParams = GradientParams(
  multX: 2.0,
  multY: 2.0,
  brightness: 1.0,
);

const effectParams = EffectParams(
  mouseInfluence: -0.2,
  scale: 1.25,
  noise: 1.5,
  bw: 0.0,
);

class CoverWallLayoutState extends State<CoverWallLayout> {
  final _focusNode = FocusNode();
  final _visualizer = VisualizerController();
  final _contextController = RouterAwareFlyoutController();
  final _contextAttachKey = GlobalKey();

  @override
  void dispose() {
    _contextController.dispose();
    _focusNode.dispose();
    super.dispose();
  }

  // The menu lists the checked scenes, the one on screen checked; without
  // the visualizer there is nothing to choose.
  Future<void> _openContextMenu(Offset localPosition) async {
    if (!mounted) return;
    final targetContext = _contextAttachKey.currentContext;

    if (targetContext == null) return;
    final provider = context.read<VisualizerScenesProvider>();
    if (!provider.showVisualizer) return;
    // The menu goes on the app's own navigator: the page's is under the
    // playback controller and the navigation bar, which would cover it.
    final navigator = Navigator.of(context, rootNavigator: true);
    final box = targetContext.findRenderObject() as RenderBox;
    final position = box.localToGlobal(
      localPosition,
      ancestor: navigator.context.findRenderObject(),
    );
    // The flyout is outside the page, so it needs the page's theme (dark over
    // a scene in light mode).
    final theme = FluentTheme.of(context);

    final s = S.of(context);
    final current = _visualizer.scenePath.value;

    _contextController.showFlyout(
      position: position,
      navigatorKey: navigator,
      builder: (_) {
        return FluentTheme(
          data: theme,
          child: MenuFlyout(
            items: [
              for (final scene in provider.scenes)
                if (scene.enabled)
                  MenuFlyoutItem(
                    leading: scene.path == current
                        ? const Icon(Symbols.check)
                        : null,
                    text: Text(scene.isCoverWall ? s.coverWall : scene.name),
                    onPressed: () => _visualizer.loadPreset(scene.path),
                  ),
            ],
          ),
        );
      },
    );
  }

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final isDark = theme.brightness.isDark;
    final shadowColor = isDark ? Colors.black : theme.accentColor.lightest;

    final r = Provider.of<ResponsiveProvider>(context);
    final isMini = r.smallerOrEqualTo(DeviceType.car, false);
    final showVisualizer = context
        .select<VisualizerScenesProvider, bool>((p) => p.showVisualizer);

    return LayoutBuilder(
      builder: (context, constraints) {
        final gridSize = calculateCoverWallGridSize(constraints);
        final crossAxisCount = (constraints.maxWidth / gridSize).ceil();
        final mainAxisCount = (constraints.maxHeight / gridSize).ceil();

        final coverArtWall = ClipRect(
          child: OverflowBox(
            alignment: Alignment.topLeft,
            maxWidth: (crossAxisCount * gridSize).toDouble(),
            maxHeight: (mainAxisCount * gridSize).toDouble(),
            child: Center(
              child: SizedBox.expand(
                child: GradientContainer(
                  gradientParams: gradientParams,
                  effectParams: effectParams,
                  color: isDark ? theme.accentColor : theme.accentColor.darkest,
                  color2: theme.accentColor.darkest.darken(0.7),
                  child: const CoverWallBackground(seed: 42, gap: 2),
                ),
              ),
            ),
          ),
        );

        final stack = Stack(
          alignment: isMini ? Alignment.centerLeft : Alignment.bottomCenter,
          children: [
            Container(
              color: isDark ? Colors.black : theme.accentColor.lightest.lighten(0.2),
            ),
            if (showVisualizer) ...[
              Positioned.fill(
                child: VisualizerSurface(controller: _visualizer),
              ),
              // The wall is the scene named by the cover wall path; Rust
              // draws nothing then, and the wall fades in over the texture.
              // Until a scene runs the page waits in the dark for the scene it
              // resumes (see `VisualizerController.waiting`); otherwise, and
              // when that fails or is slow, the wall shows, so a visualizer
              // that cannot start leaves the covers on screen instead of a
              // black page.
              Positioned.fill(
                child: ListenableBuilder(
                  listenable: Listenable.merge([
                    _visualizer.scenePath,
                    _visualizer.waiting,
                  ]),
                  builder: (context, child) {
                    final path = _visualizer.scenePath.value;
                    final wall = path == kCoverWallScenePath ||
                        (path == null && !_visualizer.waiting.value);
                    return AnimatedSwitcher(
                      duration: const Duration(milliseconds: 300),
                      child: wall ? child : const SizedBox.shrink(),
                    );
                  },
                  child: coverArtWall,
                ),
              ),
            ] else
              coverArtWall,
            Container(
              decoration: BoxDecoration(
                gradient: RadialGradient(
                  colors: [
                    shadowColor.withAlpha(isDark ? 20 : 140),
                    shadowColor.withAlpha(isDark ? 255 : 255),
                  ],
                  radius: 1.5,
                ),
              ),
              height: (mainAxisCount * gridSize).toDouble(),
            ),
            // In the lab mode a scrim darkens the visualizer, which keeps
            // its place and size behind it.
            Positioned.fill(
              child: LabModeBuilder(
                builder: (context, value, on, child) {
                  // The scrim goes with the lab's body while the lab peeks.
                  final peeking = context.select<VisualizerLabProvider, bool>(
                    (p) => p.peeking,
                  );
                  return value == 0
                      ? const SizedBox.shrink()
                      : IgnorePointer(
                          child: AnimatedOpacity(
                            opacity: peeking ? 0 : 1,
                            duration: labPeekDuration,
                            child: ColoredBox(
                              color: Colors.black.withValues(
                                alpha: 0.62 * value,
                              ),
                            ),
                          ),
                        );
                },
              ),
            ),
            // The track leaves through the top of the window, ahead of the
            // controller that follows it there.
            LabModeBuilder(
              enterDuration: const Duration(milliseconds: 200),
              builder: (context, value, on, child) => IgnorePointer(
                ignoring: on,
                child: Transform.translate(
                  offset: Offset(0, -value * constraints.maxHeight),
                  child: child,
                ),
              ),
              child: const KeepClear(child: PlayingTrack()),
            ),
            if (!isMini)
              LabModeBuilder(
                builder: (context, value, on, child) =>
                    Opacity(opacity: 1 - value, child: child),
                child: Container(
                  decoration: BoxDecoration(
                    gradient: LinearGradient(
                      begin: const Alignment(0.0, -1.0),
                      end: const Alignment(0.0, 1.0),
                      colors: [
                        shadowColor.withAlpha(0),
                        isDark
                            ? shadowColor.withAlpha(200)
                            : shadowColor.lighten(0.2).withAlpha(220),
                      ],
                    ),
                  ),
                  height: playbackControllerHeight,
                ),
              ),
            // The controller takes its place in the lab mode.
            Positioned(
              top: 0,
              left: 0,
              child: LabModeBuilder(
                builder: (context, value, on, child) => IgnorePointer(
                  ignoring: on,
                  child: Opacity(opacity: 1 - value, child: child),
                ),
                child: const BackButton(),
              ),
            ),
            // A problem of the visualizer, in the corner.
            if (showVisualizer) VisualizerProblem(controller: _visualizer),
            VisualizerLabOverlay(controller: _visualizer),
          ],
        );

        if (!isVisualizerSupported) return stack;

        // Right-clicks anywhere on the page open the menu; the playing track
        // handles its own clicks first.
        final menuArea = ContextMenuWrapper(
          onContextMenu: _openContextMenu,
          onMiddleClick: (_) {},
          contextAttachKey: _contextAttachKey,
          contextController: _contextController,
          child: stack,
        );

        // The page needs the focus for the keys; a click takes it back.
        return CallbackShortcuts(
          bindings: {
            if (showVisualizer) ...{
              const SingleActivator(LogicalKeyboardKey.arrowRight):
                  _visualizer.next,
              const SingleActivator(LogicalKeyboardKey.arrowLeft):
                  _visualizer.previous,
              const SingleActivator(LogicalKeyboardKey.keyR):
                  _visualizer.random,
            },
          },
          child: Focus(
            autofocus: showVisualizer,
            focusNode: _focusNode,
            child: Listener(
              onPointerDown: (_) => _focusNode.requestFocus(),
              child: menuArea,
            ),
          ),
        );
      },
    );
  }
}
