import 'package:go_router/go_router.dart';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:responsive_framework/responsive_framework.dart';

import '../../providers/status.dart';
import '../../utils/format_time.dart';
import '../../widgets/tile/cover_art.dart';
import '../../widgets/playback_controller/cover_wall_button.dart';
import '../../widgets/playback_controller/constants/controller_items.dart';
import '../../messages/playback.pb.dart';
import '../../providers/playback_controller.dart';

import './constants/playback_controller_height.dart';
import './like_button.dart';
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
    final s = Provider.of<PlaybackStatusProvider>(context).playbackStatus;
    final isCoverArtWall = GoRouterState.of(context).fullPath == '/cover_wall';

    final r = ResponsiveBreakpoints.of(context);

    final largeLayout = isCoverArtWall && r.smallerOrEqualTo(PHONE);
    final notReady = s?.ready == null || s?.ready == false;

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
          if (!largeLayout) NowPlaying(notReady: notReady, status: s),
          if (largeLayout)
            Transform.translate(
              offset: const Offset(0, -44),
              child: CoverArtPageProgressBar(notReady: notReady, status: s),
            ),
          ControllerButtons(notReady: notReady, status: s)
        ],
      ),
    );
  }
}

class NowPlaying extends StatelessWidget {
  const NowPlaying({
    super.key,
    required this.status,
    required this.notReady,
  });

  final PlaybackStatus? status;
  final bool notReady;

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final typography = theme.typography;

    final r = ResponsiveBreakpoints.of(context);

    final miniLayout = r.smallerOrEqualTo(TABLET);
    final hideProgress = r.isPhone;

    final progress =
        NowPlayingImplementation(notReady: notReady, status: status);

    return SizedBox.expand(
      child: Align(
        alignment: miniLayout ? Alignment.centerLeft : Alignment.center,
        child: miniLayout
            ? Row(
                children: [
                  const SizedBox(width: 16),
                  Button(
                      style: const ButtonStyle(
                        padding: WidgetStatePropertyAll(
                          EdgeInsets.all(0),
                        ),
                      ),
                      onPressed: () {
                        showCoverArtWall(context);
                      },
                      child: ClipRRect(
                        borderRadius: BorderRadius.circular(3),
                        child: CoverArt(
                          path: status?.coverArtPath,
                          hint: status != null
                              ? (
                                  status!.album,
                                  status!.artist,
                                  'Total Time ${formatTime(status!.duration)}'
                                )
                              : null,
                          size: 48,
                        ),
                      )),
                  if (r.isPhone) const SizedBox(width: 10),
                  hideProgress
                      ? Expanded(
                          child: ConstrainedBox(
                            constraints: const BoxConstraints(maxWidth: 116),
                            child: Column(
                              mainAxisAlignment: MainAxisAlignment.center,
                              crossAxisAlignment: CrossAxisAlignment.start,
                              children: [
                                Text(
                                  status?.title ?? "",
                                  overflow: TextOverflow.ellipsis,
                                ),
                                const SizedBox(height: 4),
                                Text(
                                  status?.artist ?? "",
                                  overflow: TextOverflow.ellipsis,
                                  style: typography.caption?.apply(
                                    color: theme.inactiveColor.withAlpha(160),
                                  ),
                                ),
                              ],
                            ),
                          ),
                        )
                      : progress,
                  if (r.isPhone) const SizedBox(width: 88),
                ],
              )
            : progress,
      ),
    );
  }
}

class CoverArtPageProgressBar extends StatelessWidget {
  const CoverArtPageProgressBar({
    super.key,
    required this.status,
    required this.notReady,
  });

  final PlaybackStatus? status;
  final bool notReady;

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final typography = theme.typography;

    return Column(
      mainAxisAlignment: MainAxisAlignment.center,
      children: [
        Padding(
          padding: const EdgeInsets.symmetric(horizontal: 40),
          child: Row(
            mainAxisAlignment: MainAxisAlignment.spaceBetween,
            children: [
              Text(
                formatTime(status?.progressSeconds ?? 0),
                style: typography.caption,
              ),
              Expanded(
                child: Slider(
                  value: status != null
                      ? (status?.progressPercentage ?? 0) * 100
                      : 0,
                  onChanged: status != null && !notReady
                      ? (v) => SeekRequest(
                            positionSeconds:
                                (v / 100) * (status?.duration ?? 0),
                          ).sendSignalToRust()
                      : null,
                  style: const SliderThemeData(useThumbBall: false),
                ),
              ),
              Text(
                '-${formatTime((status?.duration ?? 0) - (status?.progressSeconds ?? 0))}',
                style: typography.caption,
              ),
            ],
          ),
        ),
      ],
    );
  }
}

class NowPlayingImplementation extends StatelessWidget {
  const NowPlayingImplementation({
    super.key,
    required this.status,
    required this.notReady,
  });

  final PlaybackStatus? status;
  final bool notReady;

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final typography = theme.typography;

    return Container(
      constraints: const BoxConstraints(minWidth: 200, maxWidth: 360),
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 10),
            child: Row(
              mainAxisAlignment: MainAxisAlignment.spaceBetween,
              crossAxisAlignment: CrossAxisAlignment.end,
              children: [
                Expanded(
                  child: Text(
                    status?.title ?? "",
                    overflow: TextOverflow.ellipsis,
                    style: typography.caption,
                  ),
                ),
                Padding(
                  padding: const EdgeInsetsDirectional.only(start: 16),
                  child: LikeButton(fileId: status?.id),
                )
              ],
            ),
          ),
          Slider(
            value: status != null ? (status?.progressPercentage ?? 0) * 100 : 0,
            onChanged: status != null && !notReady
                ? (v) => SeekRequest(
                      positionSeconds: (v / 100) * (status?.duration ?? 0),
                    ).sendSignalToRust()
                : null,
            style: const SliderThemeData(useThumbBall: false),
          ),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 10),
            child: Row(
              mainAxisAlignment: MainAxisAlignment.spaceBetween,
              children: [
                Text(
                  formatTime(status?.progressSeconds ?? 0),
                  style: typography.caption,
                ),
                Text(
                  '-${formatTime((status?.duration ?? 0) - (status?.progressSeconds ?? 0))}',
                  style: typography.caption,
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class ControllerButtons extends StatefulWidget {
  const ControllerButtons({
    super.key,
    required this.notReady,
    required this.status,
  });

  final bool notReady;
  final PlaybackStatus? status;

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
    final miniLayout =
        ResponsiveBreakpoints.of(context).smallerOrEqualTo(MOBILE);

    final provider = Provider.of<PlaybackControllerProvider>(context);
    final entries = provider.entries;
    final hiddenIndex = entries.indexWhere((entry) => entry.id == 'hidden');
    final visibleEntries =
        hiddenIndex != -1 ? entries.sublist(0, hiddenIndex) : entries;
    final hiddenEntries =
        hiddenIndex != -1 ? entries.sublist(hiddenIndex + 1) : [];

    final coverArtWallLayout =
        ResponsiveBreakpoints.of(context).smallerOrEqualTo(PHONE) &&
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
          entry.controllerButtonBuilder(widget.notReady, widget.status),
        if (hiddenEntries.isNotEmpty)
          FlyoutTarget(
            controller: menuController,
            child: IconButton(
              icon: const Icon(Symbols.more_vert),
              onPressed: () {
                menuController.showFlyout(
                  builder: (context) {
                    return MenuFlyout(
                      items: [
                        for (var entry in hiddenEntries)
                          entry.flyoutEntryBuilder(
                            context,
                            widget.notReady,
                            widget.status,
                          ),
                      ],
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
