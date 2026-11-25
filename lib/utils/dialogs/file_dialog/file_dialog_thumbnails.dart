import 'dart:async';
import 'dart:ui' as ui;

import '../../../bindings/bindings.dart';
import '../../api/file_dialog.dart';

typedef FileDialogThumbnailFetcher =
    Future<FileDialogThumbnailPixels?> Function(
      FileDialogLocation location,
      int size,
    );

/// One thumbnail: an item at a version (its modification time) and a size.
class _Key {
  const _Key(this.location, this.modified, this.size);

  final FileDialogLocation location;
  final int? modified;
  final int size;

  @override
  bool operator ==(Object other) =>
      other is _Key &&
      other.location == location &&
      other.modified == modified &&
      other.size == size;

  @override
  int get hashCode => Object.hash(location, modified, size);
}

class _Job {
  _Job(this.key);

  final _Key key;
  final completer = Completer<ui.Image?>();
}

/// Thumbnails for the items the dialog shows. Requests run a few at a time,
/// the most recent first, since that is what the person is looking at; a
/// request whose item scrolled away before it started is dropped. Decoded
/// images are kept for reuse until the dialog closes.
class FileDialogThumbnails {
  FileDialogThumbnails({
    FileDialogThumbnailFetcher? fetch,
    this.maxInFlight = 6,
    this.capacity = 300,
  }) : _fetch = fetch ?? fetchFileDialogThumbnail;

  final FileDialogThumbnailFetcher _fetch;
  final int maxInFlight;
  final int capacity;

  /// Drawn images, least recently used first. A null value records that the
  /// platform draws none, so it is not asked again.
  final _cache = <_Key, ui.Image?>{};
  final _queue = <_Job>[];
  final _jobs = <_Key, _Job>{};
  int _inFlight = 0;
  bool _disposed = false;

  /// The physical size to ask for so an icon [logical] pixels wide stays
  /// sharp, rounded up to a few sizes so views share their thumbnails.
  static int requestSize(double logical, double devicePixelRatio) {
    final physical = (logical * devicePixelRatio).ceil();
    for (final size in const [64, 128, 256, 512]) {
      if (physical <= size) return size;
    }
    return 1024;
  }

  /// A clone of the thumbnail of [location] at [size], which the caller
  /// disposes, or null when the platform draws none or the request was
  /// dropped.
  Future<ui.Image?> load(
    FileDialogLocation location,
    int? modified,
    int size,
  ) async {
    final key = _Key(location, modified, size);
    if (_cache.containsKey(key)) {
      final image = _cache.remove(key);
      _cache[key] = image;
      return image?.clone();
    }
    final job = _jobs.putIfAbsent(key, () {
      final job = _Job(key);
      _queue.add(job);
      return job;
    });
    _pump();
    final image = await job.completer.future;
    return _disposed ? null : image?.clone();
  }

  /// Stop waiting for [location]'s thumbnail if it has not started.
  void drop(FileDialogLocation location, int? modified, int size) {
    final key = _Key(location, modified, size);
    final job = _jobs[key];
    if (job != null && _queue.remove(job)) {
      _jobs.remove(key);
      job.completer.complete(null);
    }
  }

  void _pump() {
    while (!_disposed && _inFlight < maxInFlight && _queue.isNotEmpty) {
      final job = _queue.removeLast();
      _inFlight++;
      unawaited(_run(job));
    }
  }

  Future<void> _run(_Job job) async {
    ui.Image? image;
    try {
      final pixels = await _fetch(job.key.location, job.key.size);
      if (pixels != null && !_disposed) image = await _decode(pixels);
    } catch (_) {
      // A thumbnail that fails to arrive leaves the type icon in place.
      image = null;
    }
    _inFlight--;
    _jobs.remove(job.key);
    if (_disposed) {
      image?.dispose();
      job.completer.complete(null);
      return;
    }
    _store(job.key, image);
    job.completer.complete(image);
    _pump();
  }

  void _store(_Key key, ui.Image? image) {
    _cache[key] = image;
    while (_cache.length > capacity) {
      final oldest = _cache.keys.first;
      _cache.remove(oldest)?.dispose();
    }
  }

  static Future<ui.Image> _decode(FileDialogThumbnailPixels pixels) async {
    final buffer = await ui.ImmutableBuffer.fromUint8List(pixels.rgba);
    final descriptor = ui.ImageDescriptor.raw(
      buffer,
      width: pixels.width,
      height: pixels.height,
      pixelFormat: ui.PixelFormat.rgba8888,
    );
    try {
      final codec = await descriptor.instantiateCodec();
      final frame = await codec.getNextFrame();
      codec.dispose();
      return frame.image;
    } finally {
      descriptor.dispose();
      buffer.dispose();
    }
  }

  void dispose() {
    _disposed = true;
    for (final job in _queue) {
      job.completer.complete(null);
    }
    _queue.clear();
    for (final image in _cache.values) {
      image?.dispose();
    }
    _cache.clear();
  }
}
