import 'package:provider/provider.dart';
import 'package:go_router/go_router.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../widgets/playback_controller/now_playing.dart';
import '../../widgets/playback_controller/cover_art_page_progress_bar.dart';
import '../../widgets/playback_controller/constants/controller_items.dart';
import '../../providers/playback_controller.dart';
import '../../providers/responsive_providers.dart';

import './constants/playback_controller_height.dart';
import './fft_visualize.dart';

class PlaybackController extends StatefulWidget {
  const PlaybackController({super.key});

  @override
  PlaybackControllerState createState() => PlaybackControllerState();
}

const scaleY = 0.9;

class PlaybackControllerState extends State<PlaybackController> {
  @override
  Widget build(BuildContext context) {
    final isCoverArtWall = GoRouterState.of(context).fullPath == '/cover_wall';

    final r = Provider.of<ResponsiveProvider>(context);

    final largeLayout = isCoverArtWall && r.smallerOrEqualTo(DeviceType.phone);

    return SizedBox(
      height: playbackControllerHeight,
      child: Stack(
        fit: StackFit.expand,
        alignment: Alignment.centerRight,
        children: <Widget>[
          SizedBox.expand(
            child: Center(
              child: Container(
                constraints:
                    const BoxConstraints(minWidth: 1200, maxWidth: 1600),
                child: Transform(
                  transform: Matrix4.identity()
                    ..scale(1.0, scaleY)
                    ..translate(0.0, (1 - scaleY) * 100),
                  child: const FFTVisualize(),
                ),
              ),
            ),
          ),
          if (!largeLayout) const NowPlaying(),
          if (largeLayout)
            Transform.translate(
              offset: const Offset(0, -44),
              child: const CoverArtPageProgressBar(),
            ),
          const ControllerButtons(),
        ],
      ),
    );
  }
}

class ControllerButtons extends StatefulWidget {
  const ControllerButtons({super.key});

  @override
  State<ControllerButtons> createState() => _ControllerButtonsState();
}

class _ControllerButtonsState extends State<ControllerButtons> {
  final menuController = FlyoutController();

  @override
  void dispose() {
    super.dispose();
    menuController.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final miniLayout = Provider.of<ResponsiveProvider>(context)
        .smallerOrEqualTo(DeviceType.mobile);

    final controllerProvider = Provider.of<PlaybackControllerProvider>(context);
    final entries = controllerProvider.entries;
    final hiddenIndex = entries.indexWhere((entry) => entry.id == 'hidden');
    final visibleEntries =
        hiddenIndex != -1 ? entries.sublist(0, hiddenIndex) : entries;
    final hiddenEntries =
        hiddenIndex != -1 ? entries.sublist(hiddenIndex + 1) : [];

    final coverArtWallLayout = Provider.of<ResponsiveProvider>(context)
            .smallerOrEqualTo(DeviceType.phone) &&
        GoRouterState.of(context).fullPath == '/cover_wall';

    final miniEntries = [controllerItems[1], controllerItems[2]];

    return Row(
      mainAxisAlignment: coverArtWallLayout
          ? MainAxisAlignment.spaceAround
          : MainAxisAlignment.end,
      children: [
        if (coverArtWallLayout) const SizedBox(width: 8),
        for (var entry in (miniLayout && !coverArtWallLayout)
            ? miniEntries
            : visibleEntries)
          entry.controllerButtonBuilder(context),
        if (hiddenEntries.isNotEmpty)
          FlyoutTarget(
            controller: menuController,
            child: IconButton(
              icon: const Icon(Symbols.more_vert),
              onPressed: () {
                menuController.showFlyout(
                  builder: (context) {
                    return Container(
                      constraints: const BoxConstraints(maxWidth: 200),
                      child: MenuFlyout(
                        items: [
                          for (var entry in hiddenEntries)
                            entry.flyoutEntryBuilder(context),
                        ],
                      ),
                    );
                  },
                );
              },
            ),
          ),
        const SizedBox(width: 8),
      ],
    );
  }
}
