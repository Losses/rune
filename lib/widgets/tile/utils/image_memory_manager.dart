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
  final Map<ImageKey, ui.Image> _syncImageCache = {};
  final Map<ImageKey, Set<ImageProxy>> _proxyReference = {};
  Timer? _cleanupTimer;

  ImageProxy requireProxy() {
    final proxy = ImageProxy(manager: this);
    return proxy;
  }

  ui.Image? getCachedImage(ImageKey key) {
    return _syncImageCache[key];
  }

  Future<ui.Image> loadImage(ImageKey key, ImageProxy proxy) {
    final set = _proxyReference[key] ?? {};
    _proxyReference[key] = set;

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

      _syncImageCache[key] = resizedImage;

      completer.complete(resizedImage);
    } catch (e) {
      completer.completeError(e);
    }
  }

  void releaseImage(ImageKey key, ImageProxy proxy) {
    if (!_proxyReference.containsKey(key)) return;

    final set = _proxyReference[key];
    if (set == null) return;
    set.remove(proxy);

    if (set.isEmpty) {
      _throttleCleanup();
    }
  }

  void _throttleCleanup() {
    if (_cleanupTimer?.isActive ?? false) return;

    _cleanupTimer = Timer(const Duration(milliseconds: 200), _cleanupCache);
  }

  void _cleanupCache() {
    // Create a list of keys to remove
    final Set<ImageKey> keysToRemove = {};

    // Iterate over each key-value pair in _proxyReference
    _proxyReference.forEach((key, set) {
      if (set.isEmpty) {
        keysToRemove.add(key); // Add the key to the list if the set is empty
      }
    });

    // After iteration, remove the empty keys from the caches
    _imageCache.removeWhere((value, _) => keysToRemove.contains(value));
    _syncImageCache.removeWhere((value, _) => keysToRemove.contains(value));
    _proxyReference.removeWhere((value, _) => keysToRemove.contains(value));
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
