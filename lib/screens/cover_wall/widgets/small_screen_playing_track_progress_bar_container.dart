import 'package:fluent_ui/fluent_ui.dart';
import 'package:rune/screens/cover_wall/widgets/cover_art_page_progress_bar.dart';

class SmallScreenPlayingTrackProgressBarContainer extends StatelessWidget {
  const SmallScreenPlayingTrackProgressBarContainer({
    super.key,
    required this.shadows,
  });

  final List<Shadow> shadows;

  @override
  Widget build(BuildContext context) {
    return Transform.translate(
      offset: const Offset(0, -16),
      child: SizedBox(
        height: 80,
        child: CoverArtPageProgressBar(shadows: shadows),
      ),
    );
  }
}
