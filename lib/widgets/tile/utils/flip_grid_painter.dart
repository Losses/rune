import 'dart:math';
import 'dart:ui' as ui;

import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/draw_image_to_canvas.dart';

class FlipGridPainter extends CustomPainter {
  final List<ui.Image?> images;
  final List<double> rotates;
  final int gridCount;
  final List<Color> fallbackColors;

  FlipGridPainter(
    this.images, {
    required this.rotates,
    required this.fallbackColors,
    this.gridCount = 3,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final paint = Paint();

    // Define the size of each grid cell based on gridSize
    final cellWidth = (size.width / gridCount).ceil();
    final cellHeight = (size.height / gridCount).ceil();

    // Loop through the grid
    for (int row = 0; row < gridCount; row++) {
      for (int col = 0; col < gridCount; col++) {
        final k = col + row * gridCount;
        final image = images[k % images.length];

        if (image == null) continue;

        // Calculate the rotation angle based on the grid position
        final thisRotate = rotates[(row * gridCount + col) % rotates.length];

        // Save the canvas state before applying transformations
        canvas.save();

        // Translate to the center of the current grid cell
        canvas.translate(
          cellWidth * (col + 0.5),
          cellHeight * (row + 0.5),
        );

        // Apply perspective and rotation transformations
        final matrix4 = Matrix4.identity()
          ..setEntry(3, 2, 0.005)
          ..rotateX(
            thisRotate >= pi / 2 ? pi - thisRotate : thisRotate,
          );

        canvas.transform(matrix4.storage);

        if (image.width == 1 && image.height == 1) {
          paint.color = fallbackColors[k];
          final dstRect = Rect.fromCenter(
            center: Offset.zero,
            width: cellWidth.toDouble(),
            height: cellHeight.toDouble(),
          );
          canvas.drawRect(dstRect, paint);
        } else {
          drawImageToCanvas(canvas, paint, image, cellWidth, cellHeight);
        }

        // Restore the canvas state
        canvas.restore();
      }
    }
  }

  bool static = false;

  @override
  bool shouldRepaint(covariant FlipGridPainter oldDelegate) {
    if (oldDelegate.gridCount != gridCount) return false;

    if (oldDelegate.images.length != images.length) return true;

    if (oldDelegate.fallbackColors.length != fallbackColors.length) {
      return true;
    }

    if (oldDelegate.fallbackColors[0] != fallbackColors[0]) {
      return true;
    }

    for (int i = 0; i < images.length; i++) {
      if (oldDelegate.images[i] != images[i]) {
        return true;
      }
    }

    final allZero = rotates.firstWhere((x) => x != 0, orElse: () => -1) == -1;

    if (allZero && !static) {
      static = true;
      return true;
    }

    if (allZero) return false;

    return true;
  }
}
