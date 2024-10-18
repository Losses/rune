import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/format_time.dart';
import '../../widgets/tile/cover_art.dart';
import '../../widgets/playback_controller/cover_wall_button.dart';
import '../../providers/status.dart';
import '../../providers/responsive_providers.dart';

import 'conrtoller_progress_bar.dart';

class NowPlaying extends StatelessWidget {
  const NowPlaying({super.key});

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final typography = theme.typography;

    final statusProvider = Provider.of<PlaybackStatusProvider>(context);
    final status = statusProvider.playbackStatus;
    final notReady = statusProvider.notReady;

    final r = Provider.of<ResponsiveProvider>(context);
    final miniLayout = r.smallerOrEqualTo(DeviceType.tablet);
    final hideProgress = r.smallerOrEqualTo(DeviceType.phone);

    final progress = ControllerProgressBar(notReady: notReady, status: status);

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
                                status.album,
                                status.artist,
                                'Total Time ${formatTime(status.duration)}'
                              )
                            : null,
                        size: 48,
                      ),
                    ),
                  ),
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
