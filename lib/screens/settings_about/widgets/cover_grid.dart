import 'dart:ui' as ui;
import 'package:fluent_ui/fluent_ui.dart';

import 'package:player/screens/settings_about/widgets/settings_mix.dart';

class CoverGrid extends StatelessWidget {
  final ui.Image image;
  final int gridCount;
  final List<double> rotates;

  const CoverGrid({
    super.key,
    required this.image,
    required this.rotates,
    this.gridCount = 3,
  });

  @override
  Widget build(BuildContext context) {
    return CustomPaint(
      painter: CoverGridPainter(
        image,
        gridCount: gridCount,
        rotates: rotates,
      ),
      // Set the size to fill the available space
      size: Size.infinite,
    );
  }
}
