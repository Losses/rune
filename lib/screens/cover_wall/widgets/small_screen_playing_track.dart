import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';

import '../../../utils/ax_shadow.dart';
import '../../../utils/format_time.dart';
import '../../../widgets/tile/cover_art.dart';
import '../../../widgets/playback_controller/constants/playback_controller_height.dart';
import '../../../providers/status.dart';

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

    final width = MediaQuery.of(context).size.width;

    return Selector<PlaybackStatusProvider,
        (String?, String?, String?, String?, double?)>(
      selector: (context, playbackStatusProvider) => (
        playbackStatusProvider.playbackStatus?.coverArtPath,
        playbackStatusProvider.playbackStatus?.artist,
        playbackStatusProvider.playbackStatus?.album,
        playbackStatusProvider.playbackStatus?.title,
        playbackStatusProvider.playbackStatus?.duration,
      ),
      builder: (context, p, child) {
        if (p.$1 == null) return Container();

        final artist = p.$2 ?? "Unknown Artist";
        final album = p.$3 ?? "Unknown Album";

        return Container(
          padding: const EdgeInsets.fromLTRB(
            48,
            48,
            48,
            playbackControllerHeight + 48,
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.center,
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              Container(
                decoration: BoxDecoration(
                  border: Border.all(color: Colors.white, width: 4),
                  boxShadow: axShadow(9),
                ),
                child: CoverArt(
                  hint: (
                    p.$3 ?? "",
                    p.$2 ?? "",
                    'Total Time ${formatTime(p.$5 ?? 0)}'
                  ),
                  key: p.$1 != null ? Key(p.$1.toString()) : null,
                  path: p.$1,
                  size: (width - 20).clamp(0, 240),
                ),
              ),
              const SizedBox(height: 24),
              Text(
                p.$4 ?? "Unknown Track",
                style: typography.subtitle?.apply(shadows: shadows),
              ),
              const SizedBox(height: 12),
              Text(
                '$artist · $album',
                style: typography.body?.apply(shadows: shadows),
              ),
            ],
          ),
        );
      },
    );
  }
}
