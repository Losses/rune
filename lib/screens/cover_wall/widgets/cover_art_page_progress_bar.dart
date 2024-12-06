import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/format_time.dart';
import '../../../utils/api/seek.dart';
import '../../../providers/status.dart';

class CoverArtPageProgressBar extends StatefulWidget {
  final List<Shadow> shadows;
  const CoverArtPageProgressBar({
    super.key,
    required this.shadows,
  });

  @override
  CoverArtPageProgressBarState createState() => CoverArtPageProgressBarState();
}

class CoverArtPageProgressBarState extends State<CoverArtPageProgressBar> {
  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final typography = theme.typography;

    final statusProvider = Provider.of<PlaybackStatusProvider>(context);
    final status = statusProvider.playbackStatus;
    final notReady = statusProvider.notReady;

    return Column(
      mainAxisAlignment: MainAxisAlignment.center,
      children: [
        Expanded(
          child: Slider(
            value: status.progressPercentage * 100,
            onChanged: !notReady ? (v) => seek(v, status) : null,
            style: const SliderThemeData(useThumbBall: false),
          ),
        ),
        Transform.translate(
          offset: const Offset(0, -24),
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 10),
            child: Row(
              mainAxisAlignment: MainAxisAlignment.spaceBetween,
              children: [
                Text(
                  formatTime(status.progressSeconds),
                  style: typography.caption
                      ?.apply(shadows: widget.shadows, fontSizeFactor: 0.9),
                ),
                Text(
                  '-${formatTime((status.duration) - (status.progressSeconds))}',
                  style: typography.caption
                      ?.apply(shadows: widget.shadows, fontSizeFactor: 0.9),
                ),
              ],
            ),
          ),
        ),
      ],
    );
  }
}
