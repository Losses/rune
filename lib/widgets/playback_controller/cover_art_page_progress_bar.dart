import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/format_time.dart';
import '../../messages/playback.pb.dart';

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
