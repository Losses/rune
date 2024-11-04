import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../../widgets/playback_controller/constants/playback_controller_height.dart';
import '../../../providers/status.dart';
import '../../../providers/responsive_providers.dart';

import 'small_screen_playing_track_cover_art_container.dart';
import 'small_screen_playing_track_command_bar_container.dart';
import 'small_screen_playing_track_progress_bar_container.dart';

class SmallScreenPlayingTrack extends StatelessWidget {
  const SmallScreenPlayingTrack({super.key});

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final isDark = theme.brightness.isDark;
    final shadowColor = isDark ? Colors.black : theme.accentColor.lightest;

    final typography = theme.typography;

    final shadows = <Shadow>[
      Shadow(color: shadowColor, blurRadius: 12),
      Shadow(color: shadowColor, blurRadius: 24),
    ];

    return DeviceTypeBuilder(
      deviceType: const [
        DeviceType.zune,
        DeviceType.car,
        DeviceType.tv,
        DeviceType.station
      ],
      builder: (context, activeBreakpoint) {
        final isZune = activeBreakpoint == DeviceType.zune;
        final isCar = activeBreakpoint == DeviceType.car;

        return Selector<PlaybackStatusProvider,
            (String?, String?, String?, String?, double?)>(
          selector: playbackStatusSelector,
          builder: (context, p, child) {
            final coverArtPath = p.$1;
            final title = p.$4;
            final duration = p.$5;

            if (coverArtPath == null) return Container();

            final artist = p.$2 ?? "Unknown Artist";
            final album = p.$3 ?? "Unknown Album";

            final result = Container(
              padding: isCar
                  ? const EdgeInsets.fromLTRB(48, 12, 12, 12)
                  : const EdgeInsets.fromLTRB(
                      12,
                      12,
                      12,
                      playbackControllerHeight + 12,
                    ),
              constraints: isCar ? null : const BoxConstraints(maxWidth: 240),
              child: Column(
                crossAxisAlignment: isCar
                    ? CrossAxisAlignment.start
                    : CrossAxisAlignment.center,
                mainAxisAlignment: MainAxisAlignment.center,
                mainAxisSize: MainAxisSize.max,
                children: [
                  if (!isZune && !isCar)
                    SmallScreenPlayingTrackCoverArtContainer(
                      album: album,
                      artist: artist,
                      duration: duration,
                      coverArtPath: coverArtPath,
                    ),
                  if (!isZune && !isCar)
                    SmallScreenPlayingTrackProgressBarContainer(
                      shadows: shadows,
                    ),
                  if (!isZune) const SizedBox(height: 8),
                  Text(
                    title ?? "Unknown Track",
                    style: typography.subtitle?.apply(shadows: shadows),
                    textAlign: TextAlign.center,
                    overflow: TextOverflow.ellipsis,
                  ),
                  const SizedBox(height: 12),
                  Text(
                    '$artist · $album',
                    style: typography.body
                        ?.apply(shadows: shadows, heightFactor: 2),
                    textAlign: TextAlign.center,
                    overflow: TextOverflow.ellipsis,
                  ),
                  if (isZune || isCar) const SizedBox(height: 12),
                  if (isZune || isCar)
                    SmallScreenPlayingTrackCommandBarContainer(
                      shadows: shadows,
                    ),
                ],
              ),
            );

            return result;
          },
        );
      },
    );
  }

  static (String?, String?, String?, String?, double?) playbackStatusSelector(
    context,
    playbackStatusProvider,
  ) =>
      (
        playbackStatusProvider.playbackStatus?.coverArtPath,
        playbackStatusProvider.playbackStatus?.artist,
        playbackStatusProvider.playbackStatus?.album,
        playbackStatusProvider.playbackStatus?.title,
        playbackStatusProvider.playbackStatus?.duration,
      );
}
