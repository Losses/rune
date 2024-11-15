import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/ax_shadow.dart';
import '../../../providers/status.dart';
import '../../../widgets/tile/cover_art.dart';
import '../../../widgets/playback_controller/constants/playback_controller_height.dart';
import '../../../generated/l10n.dart';

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
        (String?, String?, String?, String?)>(
      selector: (context, playbackStatusProvider) => (
        playbackStatusProvider.playbackStatus?.coverArtPath,
        playbackStatusProvider.playbackStatus?.artist,
        playbackStatusProvider.playbackStatus?.album,
        playbackStatusProvider.playbackStatus?.title,
      ),
      builder: (context, p, child) {
        if (p.$1 == null) return Container();
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
                  key: p.$1 != null ? Key(p.$1.toString()) : null,
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
                    p.$3 ?? S.of(context).unknownAlbum,
                    style: typography.bodyLarge?.apply(shadows: shadows),
                  ),
                  Text(
                    p.$4 ?? S.of(context).unknownTrack,
                    style: typography.subtitle?.apply(shadows: shadows),
                  ),
                  const SizedBox(height: 12),
                  Text(
                    p.$2 ?? S.of(context).unknownArtist,
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
