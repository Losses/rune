import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/format_time.dart';
import '../../widgets/tile/cover_art.dart';
import '../../widgets/playback_controller/cover_wall_button.dart';
import '../../providers/status.dart';

import '../ax_pressure.dart';
import '../tile/tile.dart';
import '../ax_reveal/ax_reveal.dart';

class NowPlayingTrackCoverArtButton extends StatefulWidget {
  const NowPlayingTrackCoverArtButton({
    super.key,
    required this.size,
  });

  final double? size;

  @override
  State<NowPlayingTrackCoverArtButton> createState() =>
      _NowPlayingTrackCoverArtButtonState();
}

class _NowPlayingTrackCoverArtButtonState
    extends State<NowPlayingTrackCoverArtButton> {
  @override
  Widget build(BuildContext context) {
    final statusProvider = Provider.of<PlaybackStatusProvider>(context);
    final status = statusProvider.playbackStatus;

    return AxPressure(
      child: AxReveal0(
        child: SizedBox(
          width: widget.size,
          height: widget.size,
          child: Tile(
            onPressed: showCoverArtWall,
            tolerateMode: true,
            child: CoverArt(
              path: status.coverArtPath,
              hint: (
                status.album ?? "",
                status.artist ?? "",
                'Total Time ${formatTime(status.duration)}'
              ),
              size: widget.size,
            ),
          ),
        ),
      ),
    );
  }
}
