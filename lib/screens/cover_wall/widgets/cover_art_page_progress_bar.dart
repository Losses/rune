import 'dart:async';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/format_time.dart';
import '../../../messages/playback.pb.dart';
import '../../../providers/status.dart';

class CoverArtPageProgressBar extends StatefulWidget {
  final List<Shadow> shadows;
  const CoverArtPageProgressBar({
    super.key,
    required this.shadows,
  });

  @override
  CoverArtPageProgressBarState createState() =>
      CoverArtPageProgressBarState();
}

class CoverArtPageProgressBarState extends State<CoverArtPageProgressBar> {
  Timer? _debounceTimer;

  void _onSeek(double value, PlaybackStatus? status) {
    if (_debounceTimer?.isActive ?? false) {
      _debounceTimer?.cancel();
    }

    _debounceTimer = Timer(const Duration(milliseconds: 100), () {
      if (status != null) {
        SeekRequest(
          positionSeconds: (value / 100) * status.duration,
        ).sendSignalToRust();
      }
    });
  }

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
            value: status != null ? status.progressPercentage * 100 : 0,
            onChanged:
                status != null && !notReady ? (v) => _onSeek(v, status) : null,
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
                  formatTime(status?.progressSeconds ?? 0),
                  style: typography.caption
                      ?.apply(shadows: widget.shadows, fontSizeFactor: 0.9),
                ),
                Text(
                  '-${formatTime((status?.duration ?? 0) - (status?.progressSeconds ?? 0))}',
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

  @override
  void dispose() {
    _debounceTimer?.cancel();
    super.dispose();
  }
}
