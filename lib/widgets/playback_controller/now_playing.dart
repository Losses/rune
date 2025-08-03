import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../widgets/playback_controller/now_playing_track_cover_art_button.dart';
import '../../providers/status.dart';
import '../../providers/responsive_providers.dart';

import 'conrtoller_progress_bar.dart';
import 'cover_wall_button.dart';
import 'track_title.dart';

class NowPlaying extends StatelessWidget {
  const NowPlaying({super.key});

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final typography = theme.typography;

    final statusProvider = Provider.of<PlaybackStatusProvider>(context);
    final status = statusProvider.playbackStatus;
    final notReady = statusProvider.notReady;
    final item = statusProvider.playingItem;

    final r = Provider.of<ResponsiveProvider>(context);
    final miniLayout = r.smallerOrEqualTo(DeviceType.tablet);
    final hideProgress = r.smallerOrEqualTo(DeviceType.phone);

    final progress = ControllerProgressBar(
      item: item,
      status: status,
      notReady: notReady,
    );

    return SizedBox.expand(
      child: Align(
        alignment: miniLayout ? Alignment.centerLeft : Alignment.center,
        child: miniLayout
            ? Row(
                children: [
                  const SizedBox(width: 16),
                  const NowPlayingTrackCoverArtButton(size: 48),
                  if (hideProgress) const SizedBox(width: 10),
                  hideProgress
                      ? Expanded(
                          child: ConstrainedBox(
                            constraints: const BoxConstraints(maxWidth: 116),
                            child: Column(
                              mainAxisAlignment: MainAxisAlignment.center,
                              crossAxisAlignment: CrossAxisAlignment.start,
                              children: [
                                TrackTitle(
                                  title: status.title ?? "",
                                  style: typography.body,
                                  onPressed: showCoverArtWall,
                                ),
                                const SizedBox(height: 4),
                                Text(
                                  status.artist ?? "",
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
