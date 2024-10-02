import 'package:player/widgets/playback_controller/constants/controller_items.dart';
import 'package:player/widgets/tile/cover_art.dart';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:responsive_framework/responsive_framework.dart';

import '../../utils/format_time.dart';
import '../../messages/playback.pb.dart';
import '../../providers/status.dart';
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
    return Consumer<PlaybackStatusProvider>(
      builder: (context, playbackStatusProvider, child) {
        final s = playbackStatusProvider.playbackStatus;

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
              NowPlaying(notReady: notReady, status: s),
              ControllerButtons(notReady: notReady, status: s)
            ],
          ),
        );
      },
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

    final tablet = ResponsiveBreakpoints.of(context).isTablet;
    final mobile = ResponsiveBreakpoints.of(context).isMobile;
    final phone = ResponsiveBreakpoints.of(context).isPhone;

    final miniLayout = tablet || mobile || phone;
    final hideProgress = phone;

    final progress = Progress(notReady: notReady, status: status);

    return SizedBox.expand(
      child: Align(
        alignment: miniLayout ? Alignment.centerLeft : Alignment.center,
        child: miniLayout
            ? Row(
                children: [
                  const SizedBox(width: 16),
                  CoverArt(
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
                  if (phone) const SizedBox(width: 10),
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
                  if (phone) const SizedBox(width: 88),
                ],
              )
            : progress,
      ),
    );
  }
}

class Progress extends StatelessWidget {
  const Progress({
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
            value: status != null ? status?.progressPercentage ?? 0 * 100 : 0,
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
    final mobile = ResponsiveBreakpoints.of(context).isMobile;
    final phone = ResponsiveBreakpoints.of(context).isPhone;

    final miniLayout = phone || mobile;

    final provider = Provider.of<PlaybackControllerProvider>(context);
    final entries = provider.entries;
    final hiddenIndex = entries.indexWhere((entry) => entry.id == 'hidden');
    final visibleEntries =
        hiddenIndex != -1 ? entries.sublist(0, hiddenIndex) : entries;
    final hiddenEntries =
        hiddenIndex != -1 ? entries.sublist(hiddenIndex + 1) : [];

    final miniEntries = [controllerItems[1], controllerItems[2]];

    return Row(
      mainAxisAlignment: MainAxisAlignment.end,
      children: [
        for (var entry in miniLayout ? miniEntries : visibleEntries)
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
