import 'dart:ui' as ui;

import 'package:fluent_ui/fluent_ui.dart';

class CoverGridPainter extends CustomPainter {
  final ui.Image image;
  final List<double> rotates;
  final int gridCount;

  CoverGridPainter(
    this.image, {
    required this.rotates,
    this.gridCount = 3,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final paint = Paint();

    // Define the size of each grid cell based on gridSize
    final cellWidth = size.width / gridCount;
    final cellHeight = size.height / gridCount;

    // Loop through the grid
    for (int row = 0; row < gridCount; row++) {
      for (int col = 0; col < gridCount; col++) {
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
          ..setEntry(3, 2, 0.005) // Perspective
          ..rotateX(thisRotate); // Rotation around X-axis

        canvas.transform(matrix4.storage);

        // Define source and destination rectangles for drawing the image
        final srcRect = Rect.fromLTWH(
          0,
          0,
          image.width.toDouble(),
          image.height.toDouble(),
        );
        final dstRect = Rect.fromCenter(
          center: Offset.zero,
          width: cellWidth,
          height: cellHeight,
        );

        // Draw the image
        canvas.drawImageRect(image, srcRect, dstRect, paint);

        // Restore the canvas state
        canvas.restore();
      }
    }
  }

  @override
  bool shouldRepaint(covariant CustomPainter oldDelegate) => true;
}
