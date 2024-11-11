import 'package:fluent_ui/fluent_ui.dart';
import 'package:rune/utils/ax_shadow.dart';
import 'package:rune/utils/format_time.dart';
import 'package:rune/widgets/tile/cover_art.dart';

class SmallScreenPlayingTrackCoverArtContainer extends StatelessWidget {
  const SmallScreenPlayingTrackCoverArtContainer({
    super.key,
    required this.album,
    required this.artist,
    required this.duration,
    required this.coverArtPath,
  });

  final String album;
  final String artist;
  final double? duration;
  final String? coverArtPath;

  @override
  Widget build(BuildContext context) {
    final width = MediaQuery.of(context).size.width;

    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 10),
      child: Container(
        decoration: BoxDecoration(
          border: Border.all(color: Colors.white, width: 4),
          boxShadow: axShadow(9),
        ),
        child: AspectRatio(
          aspectRatio: 1,
          child: CoverArt(
            hint: (album, artist, 'Total Time ${formatTime(duration ?? 0)}'),
            key: coverArtPath != null ? Key(coverArtPath.toString()) : null,
            path: coverArtPath,
            size: (width - 20).clamp(0, 240),
          ),
        ),
      ),
    );
  }
}
