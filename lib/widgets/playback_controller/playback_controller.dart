import 'package:material_symbols_icons/symbols.dart';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

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

class PlaybackControllerState extends State<PlaybackController> {
  @override
  Widget build(BuildContext context) {
    return Consumer<PlaybackStatusProvider>(
      builder: (context, playbackStatusProvider, child) {
        final theme = FluentTheme.of(context);
        final typography = theme.typography;

        final s = playbackStatusProvider.playbackStatus;

        const scaleY = 0.9;

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
              SizedBox.expand(
                child: Center(
                  child: Container(
                    constraints:
                        const BoxConstraints(minWidth: 200, maxWidth: 360),
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
                                  s != null ? s.title : "",
                                  overflow: TextOverflow.ellipsis,
                                  style: typography.caption,
                                ),
                              ),
                              Padding(
                                padding:
                                    const EdgeInsetsDirectional.only(start: 16),
                                child: LikeButton(fileId: s?.id),
                              )
                            ],
                          ),
                        ),
                        Slider(
                          value: s != null ? s.progressPercentage * 100 : 0,
                          onChanged: s != null && !notReady
                              ? (v) => SeekRequest(
                                    positionSeconds: (v / 100) * s.duration,
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
                                formatTime(s != null ? s.progressSeconds : 0),
                                style: typography.caption,
                              ),
                              Text(
                                '-${formatTime(s != null ? s.duration - s.progressSeconds : 0)}',
                                style: typography.caption,
                              ),
                            ],
                          ),
                        ),
                      ],
                    ),
                  ),
                ),
              ),
              ControllerButtons(notReady: notReady, status: s)
            ],
          ),
        );
      },
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
    final provider = Provider.of<PlaybackControllerProvider>(context);
    final entries = provider.entries;
    final hiddenIndex = entries.indexWhere((entry) => entry.id == 'hidden');
    final visibleEntries =
        hiddenIndex != -1 ? entries.sublist(0, hiddenIndex) : entries;
    final hiddenEntries =
        hiddenIndex != -1 ? entries.sublist(hiddenIndex + 1) : [];

    return Row(
      mainAxisAlignment: MainAxisAlignment.end,
      children: [
        for (var entry in visibleEntries)
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
                              context, widget.notReady, widget.status),
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
