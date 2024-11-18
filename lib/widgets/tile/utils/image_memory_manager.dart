import 'dart:ui' as ui;
import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import '../../../utils/load_and_resize_image.dart';

import 'image_proxy.dart';

final Uint8List blankBytes = const Base64Codec()
    .decode("R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7");

final emptyKey = ImageKey('', 0);

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
    if (key.path == '') {
      key = emptyKey;
    }

    final set = _proxyReference[key] ?? {};
    _proxyReference[key] = set;

    if (_imageCache.containsKey(key)) {
      set.add(proxy);
      return _imageCache[key]!.future;
    } else {
      final completer = Completer<ui.Image>();
      _imageCache[key] = completer;

      set.add(proxy);

      if (key.path == '') {
        _loadAndCacheEmptyImage(completer);
      } else {
        _loadAndCacheImage(key, completer);
      }

      return completer.future;
    }
  }

  Future<void> _loadAndCacheEmptyImage(
    Completer<ui.Image> completer,
  ) async {
    final ui.Codec codec = await ui.instantiateImageCodecFromBuffer(
      await ui.ImmutableBuffer.fromUint8List(blankBytes),
    );

    final ui.FrameInfo frameInfo = await codec.getNextFrame();
    final ui.Image emptylImage = frameInfo.image;

    _syncImageCache[emptyKey] = emptylImage;

    completer.complete(emptylImage);
  }

  Future<void> _loadAndCacheImage(
    ImageKey key,
    Completer<ui.Image> completer,
  ) async {
    try {
      final ui.Image resizedImage =
          await loadAndResizeImage(key.path, key.size);

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
