import 'package:flutter/services.dart';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../utils/l10n.dart';
import '../../../utils/color_brightness.dart';
import '../../../utils/visualizer_supported.dart';
import '../../../utils/router/router_aware_flyout_controller.dart';
import '../../../widgets/context_menu_wrapper.dart';
import '../../../widgets/visualizer/visualizer_controller.dart';
import '../../../widgets/visualizer/visualizer_surface.dart';
import '../../../widgets/cover_wall_background/gradient_container.dart';
import '../../../widgets/cover_wall_background/cover_wall_background.dart';
import '../../../widgets/cover_wall_background/utils/calculate_cover_wall_size.dart';
import '../../../widgets/playback_controller/constants/playback_controller_height.dart';
import '../../../providers/responsive_providers.dart';
import '../../../providers/cover_wall_display.dart';

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
  void initState() {
    super.initState();
    _contextController.controller.addListener(_handleContextControllerUpdate);
  }

  @override
  void dispose() {
    _contextController.controller
        .removeListener(_handleContextControllerUpdate);
    _contextController.dispose();
    _focusNode.dispose();
    super.dispose();
  }

  // The visualizer waits with switching presets while the menu is open.
  void _handleContextControllerUpdate() {
    _visualizer.setMenuOpen(_contextController.isOpen);
  }

  void _openContextMenu(Offset localPosition) {
    if (!mounted) return;
    final targetContext = _contextAttachKey.currentContext;

    if (targetContext == null) return;
    final box = targetContext.findRenderObject() as RenderBox;
    final position = box.localToGlobal(
      localPosition,
      ancestor: Navigator.of(context).context.findRenderObject(),
    );

    final s = S.of(context);
    final provider = context.read<CoverWallDisplayProvider>();
    final showVisualizer = provider.showVisualizer;

    MenuFlyoutItem displayItem(CoverWallDisplay display, String title) {
      final active = provider.display == display;
      return MenuFlyoutItem(
        leading: Icon(
          active ? Symbols.radio_button_checked : Symbols.radio_button_unchecked,
        ),
        text: Text(title),
        onPressed: () => provider.setDisplay(display),
      );
    }

    _contextController.showFlyout(
      position: position,
      builder: (_) {
        return MenuFlyout(
          items: [
            displayItem(CoverWallDisplay.covers, s.coverWallDisplayCovers),
            displayItem(
              CoverWallDisplay.visualizer,
              s.coverWallDisplayVisualizer,
            ),
            if (showVisualizer) ...[
              const MenuFlyoutSeparator(),
              MenuFlyoutItem(
                text: ConstrainedBox(
                  constraints: const BoxConstraints(maxWidth: 280),
                  child: Text(
                    _visualizer.infoText(s),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
                onPressed: null,
              ),
              const MenuFlyoutSeparator(),
              MenuFlyoutItem(
                leading: const Icon(Symbols.skip_previous),
                text: Text(s.visualizerPrevious),
                onPressed: _visualizer.previous,
              ),
              MenuFlyoutItem(
                leading: const Icon(Symbols.skip_next),
                text: Text(s.visualizerNext),
                onPressed: _visualizer.next,
              ),
              MenuFlyoutItem(
                leading: const Icon(Symbols.shuffle),
                text: Text(s.visualizerRandom),
                onPressed: _visualizer.random,
              ),
              const MenuFlyoutSeparator(),
              MenuFlyoutItem(
                leading: Icon(
                  _visualizer.locked ? Symbols.lock_open : Symbols.lock,
                ),
                text: Text(
                  _visualizer.locked
                      ? s.visualizerUnlockPreset
                      : s.visualizerLockPreset,
                ),
                onPressed: _visualizer.toggleLock,
              ),
            ],
          ],
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
        .select<CoverWallDisplayProvider, bool>((p) => p.showVisualizer);

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
            if (showVisualizer)
              Positioned.fill(
                child: VisualizerSurface(controller: _visualizer),
              )
            else
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
            const PlayingTrack(),
            if (!isMini)
              Container(
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
            const Positioned(
              top: 0,
              left: 0,
              child: BackButton(),
            )
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
