import 'dart:ui' as ui;
import 'package:http/http.dart' as http;
import 'dart:typed_data';

Future<ui.Image> loadAndResizeImage(String path, int size) async {
  // Determine if the path is remote or local
  bool isRemote = path.startsWith('http://') || path.startsWith('https://');

  late ui.Codec codec;
  if (isRemote) {
    // For remote images, fetch the data from URL
    final response = await http.get(Uri.parse(path));
    if (response.statusCode != 200) {
      throw Exception('Failed to load image from remote URL: $path');
    }

    final Uint8List bytes = response.bodyBytes;
    codec = await ui.instantiateImageCodec(bytes);
  } else {
    // For local files, load from file path
    codec = await ui.instantiateImageCodecFromBuffer(
      await ui.ImmutableBuffer.fromFilePath(path),
    );
  }

  final ui.FrameInfo frameInfo = await codec.getNextFrame();
  final ui.Image originalImage = frameInfo.image;

  // Calculate the scale to cover the target size
  final double scale = (originalImage.width > originalImage.height)
      ? size / originalImage.height
      : size / originalImage.width;

  // Calculate the new size
  final int newWidth = (originalImage.width * scale).ceil();
  final int newHeight = (originalImage.height * scale).ceil();

  // Load the image again with the target size
  late ui.Codec newCodec;
  if (isRemote) {
    // For remote images, fetch again
    final response = await http.get(Uri.parse(path));
    final Uint8List bytes = response.bodyBytes;
    newCodec = await ui.instantiateImageCodec(
      bytes,
      targetWidth: newWidth,
      targetHeight: newHeight,
    );
  } else {
    // For local files
    newCodec = await ui.instantiateImageCodecFromBuffer(
      await ui.ImmutableBuffer.fromFilePath(path),
      targetWidth: newWidth,
      targetHeight: newHeight,
    );
  }

  final ui.FrameInfo newFrameInfo = await newCodec.getNextFrame();
  final ui.Image resizedImage = newFrameInfo.image;

  return resizedImage;
}
