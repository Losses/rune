import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../providers/status.dart';
import '../../utils/format_time.dart';
import '../../utils/playing_item.dart';
import '../../utils/api/seek.dart';
import '../../widgets/playback_controller/like_button.dart';

import '../../providers/playback_controller.dart';
import '../../providers/responsive_providers.dart';

import 'cover_wall_button.dart';
import 'track_title.dart';

class ControllerProgressBar extends StatelessWidget {
  const ControllerProgressBar({
    super.key,
    required this.item,
    required this.status,
    required this.notReady,
  });

  final PlayingItem? item;
  final PlaybackStatusState? status;
  final bool notReady;

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final typography = theme.typography;

    final controllerProvider = Provider.of<PlaybackControllerProvider>(context);
    final entries = controllerProvider.entries;
    final hiddenIndex = entries.indexWhere((entry) => entry.id == 'hidden');
    final reduceCount = (hiddenIndex - 6).clamp(0, 5);

    return SmallerOrEqualTo(
      deviceType: DeviceType.tablet,
      builder: (context, isTable) {
        return Container(
          constraints: BoxConstraints(
              minWidth: 200, maxWidth: isTable ? 320 : 360 - reduceCount * 40),
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
                      child: TrackTitle(
                        title: status?.title ?? "",
                        style: typography.caption,
                        onPressed: showCoverArtWall,
                      ),
                    ),
                    Padding(
                      padding: const EdgeInsetsDirectional.only(start: 16),
                      child: LikeButton(item: item),
                    )
                  ],
                ),
              ),
              Slider(
                value: status != null
                    ? (status?.progressPercentage ?? 0) * 100
                    : 0,
                onChanged: (value) {
                  if (status != null && !notReady) {
                    seek(value, status);
                  }
                },
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
      },
    );
  }
}
