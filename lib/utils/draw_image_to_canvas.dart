import 'dart:ui' as ui;
import 'package:fluent_ui/fluent_ui.dart';

drawImageToCanvas(
  Canvas canvas,
  Paint paint,
  ui.Image image,
  int cellWidth,
  int cellHeight,
) {
  final imageAspectRatio = image.width / image.height;
  final cellAspectRatio = cellWidth / cellHeight;

  // Define source rectangle for the cover effect
  Rect srcRect;
  if (imageAspectRatio > cellAspectRatio) {
    final scale = image.height / cellHeight;
    final scaledWidth = cellWidth * scale;
    final dx = (image.width - scaledWidth) / 2;
    srcRect = Rect.fromLTWH(dx, 0, scaledWidth, image.height.toDouble());
  } else {
    // Image is wider than the cell
    final scale = image.width / cellWidth;
    final scaledHeight = cellHeight * scale;
    final dy = (image.height - scaledHeight) / 2;
    srcRect = Rect.fromLTWH(0, dy, image.width.toDouble(), scaledHeight);
  }

  // Define destination rectangle
  final dstRect = Rect.fromCenter(
    center: Offset.zero,
    width: cellWidth.toDouble(),
    height: cellHeight.toDouble(),
  );

  // Draw the image
  canvas.drawImageRect(image, srcRect, dstRect, paint);
}
