import 'dart:ui' as ui;
import 'package:fluent_ui/fluent_ui.dart';

import 'package:player/screens/settings_about/widgets/settings_mix.dart';

class CoverGrid extends StatelessWidget {
  final List<ui.Image?> images;
  final int gridCount;
  final List<double> rotates;

  const CoverGrid({
    super.key,
    required this.images,
    required this.rotates,
    this.gridCount = 3,
  });

  @override
  Widget build(BuildContext context) {
    if (images.isEmpty) return Container();

    return CustomPaint(
      painter: CoverGridPainter(
        images,
        gridCount: gridCount,
        rotates: rotates,
      ),
      // Set the size to fill the available space
      size: Size.infinite,
    );
  }
}
