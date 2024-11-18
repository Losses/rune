import 'dart:ui' as ui;

Future<ui.Image> loadAndResizeImage(String path, int size) async {
  // Load the image from file
  final ui.Codec codec = await ui.instantiateImageCodecFromBuffer(
    await ui.ImmutableBuffer.fromFilePath(path),
  );

  final ui.FrameInfo frameInfo = await codec.getNextFrame();
  final ui.Image originalImage = frameInfo.image;

  // Calculate the scale to cover the target size
  final double scale = (originalImage.width > originalImage.height)
      ? size / originalImage.height
      : size / originalImage.width;

  // Calculate the new size
  final int newWidth = (originalImage.width * scale).ceil();
  final int newHeight = (originalImage.height * scale).ceil();

  // Load the image from file again
  final ui.Codec newCodec = await ui.instantiateImageCodecFromBuffer(
    await ui.ImmutableBuffer.fromFilePath(path),
    targetWidth: newWidth,
    targetHeight: newHeight,
  );

  final ui.FrameInfo newFrameInfo = await newCodec.getNextFrame();
  final ui.Image resizedImage = newFrameInfo.image;

  return resizedImage;
}
