import 'dart:ui' as ui;

import 'image_memory_manager.dart';

class ImageProxy {
  final ImageMemoryManager manager;
  final Set<ImageKey> requestedImages = {};

  ImageProxy({required this.manager});

  Future<ui.Image> requestImage(String path, int size) {
    final key = ImageKey(path, size);
    requestedImages.add(key);
    return manager.loadImage(key, this);
  }

  ui.Image? getCachedImage(String path, int size) {
    final key = ImageKey(path, size);
    return manager.getCachedImage(key);
  }

  void dispose() {
    for (var key in requestedImages) {
      manager.releaseImage(key, this);
    }
    requestedImages.clear();
  }
}
