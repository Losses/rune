import 'dart:io';

import 'package:get_storage/get_storage.dart';
import 'package:macos_secure_bookmarks/macos_secure_bookmarks.dart';

import '../settings_manager.dart';

import '../../utils/rune_log.dart';

class MacSecureManager {
  static const storageKey = 'rune-secure-bookmarks';

  static final MacSecureManager _instance = MacSecureManager._internal();
  factory MacSecureManager() => _instance;

  late GetStorage _storage;
  bool _initialized = false;
  Future<void>? completed;

  MacSecureManager._internal() {
    completed = _init();
  }

  Future<void> _init() async {
    if (!isApplePlatform()) return;
    if (_initialized) return;

    final path = await getSettingsPath();
    info$("Initializing secure bookmarks at: $path");

    _storage = GetStorage(storageKey, path);

    await _storage.initStorage;

    await loadBookmark();

    _initialized = true;
  }

  static isApplePlatform() {
    return Platform.isMacOS || Platform.isIOS;
  }

  Future<void> saveBookmark(String dir) async {
    if (!isApplePlatform()) return;

    await completed;
    final secureBookmarks = SecureBookmarks();

    final bookmark = await secureBookmarks.bookmark(Directory(dir));
    await _storage.write(dir, bookmark);
  }

  Future<void> loadBookmark() async {
    if (!isApplePlatform()) return;

    final secureBookmarks = SecureBookmarks();

    final dirs = _storage.getKeys<Iterable<dynamic>>().toList();

    for (final dir in dirs) {
      // A folder that was deleted, or sits on a drive that is not plugged in,
      // cannot be resolved. Skip it so that it does not stop Rune from
      // starting, and keep the bookmark in case the folder comes back.
      try {
        final resolvedFile = await secureBookmarks.resolveBookmark(
          _storage.read(dir),
          isDirectory: true,
        );
        await secureBookmarks.startAccessingSecurityScopedResource(
          resolvedFile,
        );
      } catch (e) {
        warn$("Skipping secure bookmark for $dir: $e");
      }
    }
  }
}
