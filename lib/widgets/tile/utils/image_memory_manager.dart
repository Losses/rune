import 'dart:ui' as ui;
import 'dart:async';

import 'image_proxy.dart';

class ImageMemoryManager {
  static final ImageMemoryManager _instance = ImageMemoryManager._internal();

  factory ImageMemoryManager() {
    return _instance;
  }

  ImageMemoryManager._internal();

  final Map<ImageKey, Completer<ui.Image>> _imageCache = {};
  final Map<ImageKey, Set<ImageProxy>> _referenceCount = {};

  ImageProxy requireProxy() {
    final proxy = ImageProxy(manager: this);

    return proxy;
  }

  Future<ui.Image> loadImage(ImageKey key, ImageProxy proxy) {
    final set = _referenceCount[key] ?? {};
    _referenceCount[key] = set;

    if (_imageCache.containsKey(key)) {
      set.add(proxy);
      return _imageCache[key]!.future;
    } else {
      final completer = Completer<ui.Image>();
      _imageCache[key] = completer;

      set.add(proxy);

      _loadAndCacheImage(key, completer);

      return completer.future;
    }
  }

  Future<void> _loadAndCacheImage(
    ImageKey key,
    Completer<ui.Image> completer,
  ) async {
    try {
      // Load the image from file
      final ui.Codec codec = await ui.instantiateImageCodecFromBuffer(
        await ui.ImmutableBuffer.fromFilePath(key.path),
      );

      final ui.FrameInfo frameInfo = await codec.getNextFrame();
      final ui.Image originalImage = frameInfo.image;

      // Calculate the scale to cover the target size
      final double scale = (originalImage.width > originalImage.height)
          ? key.size / originalImage.height
          : key.size / originalImage.width;

      // Calculate the new size
      final int newWidth = (originalImage.width * scale).ceil();
      final int newHeight = (originalImage.height * scale).ceil();

      // Load the image from file
      final ui.Codec newCodec = await ui.instantiateImageCodecFromBuffer(
        await ui.ImmutableBuffer.fromFilePath(key.path),
        targetWidth: newWidth,
        targetHeight: newHeight,
      );

      final ui.FrameInfo newFrameInfo = await newCodec.getNextFrame();
      final ui.Image resizedImage = newFrameInfo.image;
      completer.complete(resizedImage);
    } catch (e) {
      completer.completeError(e);
    }
  }

  void releaseImage(ImageKey key, ImageProxy proxy) {
    if (!_referenceCount.containsKey(key)) return;

    final set = _referenceCount[key];
    if (set == null) return;
    set.remove(proxy);

    if (set.isEmpty) {
      _imageCache.remove(key);
      _referenceCount.remove(key);
    }
  }
}

class ImageKey {
  final String path;
  final int size;

  ImageKey(this.path, this.size);

  @override
  bool operator ==(Object other) =>
      identical(this, other) ||
      other is ImageKey &&
          runtimeType == other.runtimeType &&
          path == other.path &&
          size == other.size;

  @override
  int get hashCode => path.hashCode ^ size.hashCode;
}
