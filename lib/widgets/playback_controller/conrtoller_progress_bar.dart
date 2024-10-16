import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';

import '../../utils/format_time.dart';
import '../../widgets/playback_controller/like_button.dart';
import '../../messages/playback.pb.dart';
import '../../providers/playback_controller.dart';
import '../../providers/responsive_providers.dart';

class ControllerProgressBar extends StatelessWidget {
  const ControllerProgressBar({
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

    final controllerProvider = Provider.of<PlaybackControllerProvider>(context);
    final entries = controllerProvider.entries;
    final hiddenIndex = entries.indexWhere((entry) => entry.id == 'hidden');
    final reduceCount = (hiddenIndex - 6).clamp(0, 5);

    return SmallerOrEqualTo(
        breakpoint: DeviceType.tablet,
        builder: (context, isTable) {
          return Container(
            constraints:
                BoxConstraints(minWidth: 200, maxWidth: isTable ? 320 : 360 - reduceCount * 40),
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
        });
  }
}
