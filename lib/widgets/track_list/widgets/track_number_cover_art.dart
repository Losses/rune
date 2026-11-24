import 'package:fluent_ui/fluent_ui.dart';
import 'track_number_painter.dart';

class TrackNumberCoverArt extends StatelessWidget {
  final int? diskNumber;
  final int trackNumber;

  const TrackNumberCoverArt({
    super.key,
    required this.diskNumber,
    required this.trackNumber,
  });

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    return AspectRatio(
      aspectRatio: 1,
      child: CustomPaint(
        painter: TrackNumberPainter(
          number1: diskNumber,
          number2: trackNumber,
          color: theme.resources.textFillColorPrimary,
        ),
      ),
    );
  }
}
