import 'dart:ui' as ui;

import 'process_cover_art_path.dart';

/// Decodes an image to cover a square of [size] pixels. The caller owns the
/// returned image and must dispose it when it is no longer needed.
Future<ui.Image> loadAndResizeImage(String path, int size) async {
  path = await processCoverArtPath(path);
  final buffer = await ui.ImmutableBuffer.fromFilePath(path);
  ui.ImageDescriptor? descriptor;
  ui.Codec? codec;
  try {
    // The descriptor exposes intrinsic dimensions without decoding a full-size
    // image solely to determine the target size.
    descriptor = await ui.ImageDescriptor.encoded(buffer);
    final scale = descriptor.width > descriptor.height
        ? size / descriptor.height
        : size / descriptor.width;
    codec = await descriptor.instantiateCodec(
      targetWidth: (descriptor.width * scale).ceil(),
      targetHeight: (descriptor.height * scale).ceil(),
    );
    return (await codec.getNextFrame()).image;
  } finally {
    codec?.dispose();
    descriptor?.dispose();
    buffer.dispose();
  }
}
