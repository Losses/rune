import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/format_time.dart';
import '../../widgets/tile/cover_art.dart';
import '../../widgets/playback_controller/cover_wall_button.dart';
import '../../providers/status.dart';

class NowPlayingTrackCoverArtButton extends StatelessWidget {
  const NowPlayingTrackCoverArtButton({
    super.key,
    required this.size,
  });

  final double? size;

  @override
  Widget build(BuildContext context) {
    final statusProvider = Provider.of<PlaybackStatusProvider>(context);
    final status = statusProvider.playbackStatus;

    return Button(
      style: const ButtonStyle(
        padding: WidgetStatePropertyAll(
          EdgeInsets.all(0),
        ),
      ),
      onPressed: () {
        showCoverArtWall();
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
          size: size,
        ),
      ),
    );
  }
}
