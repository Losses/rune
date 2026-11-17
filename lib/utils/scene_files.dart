import 'dart:async';
import 'dart:io';

import 'package:flutter/services.dart';
import 'package:path/path.dart' as p;

import 'rune_log.dart';

/// Scene files (`.runescene`) the system asked Rune to open.
///
/// Each platform delivers them its own way (command line, a file association
/// event, an intent), but the native runner always hands the local paths to
/// Dart through the `ci.not.rune/scene_files` channel:
///
/// - native to Dart: `open` with a list of absolute paths;
/// - Dart to native: `takePending`, which returns the paths that arrived
///   before Dart was listening and clears them;
/// - Dart to native: `copiesDir`, which returns the folder the runner copies
///   the files into to hand them over, each copy in a folder of its own
///   (`<id>/<file>`). Only a runner that makes such copies answers it
///   (Android and iOS); the others hand over the user's own files.
class SceneFiles {
  static const _extension = '.runescene';
  static const _channel = MethodChannel('ci.not.rune/scene_files');

  static final _controller = StreamController<String>.broadcast(
    onListen: _flush,
  );

  // Paths that arrived while nobody was subscribed to [opened]. A broadcast
  // stream drops events without listeners, so they wait here.
  static final _buffer = <String>[];

  static bool _started = false;

  /// Paths of scene files to import, in the order they arrived.
  static Stream<String> get opened => _controller.stream;

  /// Call once at startup with main's arguments: Windows and Linux pass the
  /// file that launched the app on the command line. Then listens to the
  /// platform channel and asks for files that arrived before.
  static Future<void> start(List<String> arguments) async {
    if (_started) return;
    _started = true;

    for (final argument in arguments) {
      if (argument.toLowerCase().endsWith(_extension) &&
          File(argument).existsSync()) {
        _emit(argument);
      }
    }

    _channel.setMethodCallHandler((call) async {
      if (call.method == 'open') {
        for (final path in (call.arguments as List).cast<String>()) {
          _emit(path);
        }
      }
    });

    try {
      final pending = await _channel.invokeListMethod<String>('takePending');
      pending?.forEach(_emit);
    } on MissingPluginException {
      // This platform has no channel, so no file can arrive late.
    } on PlatformException {
      // The native side could not hand the paths over; nothing to import.
    }
  }

  /// Imports the opened file [path] with [run], then deletes the copy the
  /// runner made of it. [run] reports the outcome to the user itself, so that
  /// the report never waits for the cleanup, and answers whether the import is
  /// over. While it is not (no answer came in time), the file may still be
  /// read, so the copy stays.
  static Future<void> importOpened(
    String path,
    Future<bool> Function(String path) run,
  ) async {
    if (await run(path)) await _dropCopy(path);
  }

  /// Deletes the copy the runner made of the opened file [path], with the
  /// folder it made for it. A path outside the runner's folder of copies is
  /// the user's own file and stays as it is.
  static Future<void> _dropCopy(String path) async {
    final copies = await _copiesDir();
    if (copies == null) return;
    final folder = p.dirname(p.normalize(p.absolute(path)));
    if (!p.equals(p.dirname(folder), copies)) return;
    try {
      await Directory(folder).delete(recursive: true);
    } catch (e) {
      error$('Failed to delete the copy of $path: $e');
    }
  }

  static Future<String?> _copiesDir() async {
    try {
      return await _channel.invokeMethod<String>('copiesDir');
    } on MissingPluginException {
      // This runner makes no copies.
      return null;
    } on PlatformException {
      // The native side could not name its folder; nothing is deleted.
      return null;
    }
  }

  static void _emit(String path) {
    if (_controller.hasListener) {
      _controller.add(path);
    } else {
      _buffer.add(path);
    }
  }

  static void _flush() {
    if (_buffer.isEmpty) return;
    final paths = List<String>.of(_buffer);
    _buffer.clear();
    // onListen runs before the new subscriber is attached, so deliver on a
    // microtask.
    scheduleMicrotask(() {
      for (final path in paths) {
        _controller.add(path);
      }
    });
  }
}
