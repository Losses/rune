import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/ax_shadow.dart';
import '../../../providers/status.dart';
import '../../../utils/format_time.dart';
import '../../../utils/playing_item.dart';
import '../../../widgets/tile/cover_art.dart';
import '../../../widgets/playback_controller/constants/playback_controller_height.dart';
import '../../../utils/l10n.dart';

class LargeScreenPlayingTrack extends StatelessWidget {
  const LargeScreenPlayingTrack({super.key});

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final isDark = theme.brightness.isDark;
    final shadowColor = isDark ? Colors.black : theme.accentColor.lightest;

    final shadows = <Shadow>[
      Shadow(color: shadowColor, blurRadius: 12),
      Shadow(color: shadowColor, blurRadius: 24),
    ];

    final Typography typography = theme.typography;

    return Selector<PlaybackStatusProvider,
        (String, String, String, String, double, PlayingItem?)>(
      selector: (context, playbackStatusProvider) => (
        playbackStatusProvider.playbackStatus.coverArtPath ?? "",
        playbackStatusProvider.playbackStatus.artist ?? "",
        playbackStatusProvider.playbackStatus.album ?? "",
        playbackStatusProvider.playbackStatus.title ?? "",
        playbackStatusProvider.playbackStatus.duration,
        playbackStatusProvider.playingItem,
      ),
      builder: (context, p, child) {
        if (p.$6 == null) return Container();
        return Container(
          padding: const EdgeInsets.fromLTRB(
              48, 48, 48, playbackControllerHeight + 48),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.end,
            mainAxisAlignment: MainAxisAlignment.start,
            children: [
              Container(
                decoration: BoxDecoration(
                  border: Border.all(color: Colors.white, width: 4),
                  boxShadow: axShadow(9),
                ),
                child: CoverArt(
                  hint: (p.$3, p.$2, 'Total Time ${formatTime(p.$5)}'),
                  key: p.$1.isNotEmpty ? Key(p.$1.toString()) : null,
                  path: p.$1,
                  size: 120,
                ),
              ),
              const SizedBox(width: 24),
              Column(
                mainAxisAlignment: MainAxisAlignment.end,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    p.$3.isEmpty ? S.of(context).unknownAlbum : p.$3,
                    style: typography.bodyLarge?.apply(shadows: shadows),
                  ),
                  Text(
                    p.$4.isEmpty ? S.of(context).unknownTrack : p.$4,
                    style: typography.subtitle?.apply(shadows: shadows),
                  ),
                  const SizedBox(height: 12),
                  Text(
                    p.$2.isEmpty ? S.of(context).unknownArtist : p.$2,
                    style: typography.body?.apply(shadows: shadows),
                  ),
                  const SizedBox(height: 28),
                ],
              ),
            ],
          ),
        );
      },
    );
  }
}
