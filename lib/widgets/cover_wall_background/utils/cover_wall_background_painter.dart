import 'dart:ui' as ui;
import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/draw_image_to_canvas.dart';

import 'random_grid_placement.dart';

class CoverWallBackgroundPainter extends CustomPainter {
  final int gridSize;
  final int gap;
  final List<ui.Image?> images;
  final List<List<RandomGridPlacement>> grid;

  CoverWallBackgroundPainter({
    required this.gridSize,
    required this.gap,
    required this.images,
    required this.grid,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final paint = Paint();

    for (int imageIndex = 0; imageIndex < grid.length; imageIndex += 1) {
      final image = images[imageIndex];

      if (image == null) return;

      for (final gridUnit in grid[imageIndex]) {
        canvas.save();

        final x = (gridSize + gap) * gridUnit.col +
            ((gridSize + gap) * (gridUnit.size / 2));
        final y = (gridSize + gap) * gridUnit.row +
            ((gridSize + gap) * (gridUnit.size / 2));

        canvas.translate(x.toDouble(), y.toDouble());

        final drawSize = gridSize * gridUnit.size + (gridUnit.size - 1) * gap;

        drawImageToCanvas(
          canvas,
          paint,
          image,
          drawSize,
          drawSize,
        );
        canvas.restore();
      }
    }
  }

  @override
  bool shouldRepaint(CoverWallBackgroundPainter oldDelegate) {
    if (gridSize != oldDelegate.gridSize) return true;
    if (gap != oldDelegate.gap) return true;

    if (images.length != oldDelegate.images.length) return true;

    for (int i = 0; i < images.length; i++) {
      if (images[i] != oldDelegate.images[i]) return true;
    }

    if (grid.length != oldDelegate.grid.length) return true;

    for (int i = 0; i < grid.length; i++) {
      final currentList = grid[i];
      final oldList = oldDelegate.grid[i];

      if (currentList.length != oldList.length) return true;

      for (int j = 0; j < currentList.length; j++) {
        if (currentList[j] != oldList[j]) return true;
      }
    }

    return false;
  }
}
