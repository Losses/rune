import 'package:player/widgets/playback_controller/like_button.dart';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/format_time.dart';
import '../../widgets/playback_controller/next_button.dart';
import '../../widgets/playback_controller/playlist_button.dart';
import '../../widgets/playback_controller/previous_button.dart';
import '../../widgets/playback_controller/cover_wall_button.dart';
import '../../widgets/playback_controller/play_pause_button.dart';
import '../../widgets/playback_controller/playback_mode_button.dart';
import '../../widgets/playback_controller/constants/playback_controller_height.dart';
import '../../messages/playback.pb.dart';
import '../../providers/status.dart';

import 'fft_visualize.dart';

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
                              Text(
                                s != null ? s.title : "",
                                overflow: TextOverflow.ellipsis,
                                style: typography.caption,
                              ),
                              LikeButton(fileId: s?.id),
                            ],
                          ),
                        ),
                        Slider(
                          value: s != null ? s.progressPercentage * 100 : 0,
                          onChanged: s != null && !notReady
                              ? (v) => SeekRequest(
                                      positionSeconds: (v / 100) * s.duration)
                                  .sendSignalToRust()
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
              Row(
                mainAxisAlignment: MainAxisAlignment.end,
                children: [
                  PreviousButton(
                    disabled: notReady,
                  ),
                  PlayPauseButton(
                      disabled: notReady, state: s?.state ?? "Stopped"),
                  NextButton(
                    disabled: notReady,
                  ),
                  const PlaybackModeButton(),
                  PlaylistButton(),
                  const CoverWallButton(),
                  const SizedBox(width: 8),
                ],
              )
            ],
          ),
        );
      },
    );
  }
}
