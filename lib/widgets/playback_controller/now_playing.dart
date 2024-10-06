import 'package:fluent_ui/fluent_ui.dart';
import 'package:responsive_framework/responsive_framework.dart';

import '../../utils/format_time.dart';
import '../../widgets/tile/cover_art.dart';
import '../../widgets/playback_controller/cover_wall_button.dart';
import '../../widgets/playback_controller/now_playing_implementation.dart';
import '../../messages/playback.pb.dart';

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
                  if (hideProgress) const SizedBox(width: 10),
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
                  if (hideProgress) const SizedBox(width: 88),
                ],
              )
            : progress,
      ),
    );
  }
}
