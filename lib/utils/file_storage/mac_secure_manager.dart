import 'dart:io';

import 'package:get_storage/get_storage.dart';
import 'package:macos_secure_bookmarks/macos_secure_bookmarks.dart';
import 'package:rune/utils/rune_log.dart';

import '../settings_manager.dart';

const storageKey = 'rune-secure-bookmarks';

class MacSecureManager {
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

    final bookmarks = _storage.getValues<Iterable<dynamic>>().toList();

    for (final bookmark in bookmarks) {
      final resolvedFile =
          await secureBookmarks.resolveBookmark(bookmark, isDirectory: true);
      await secureBookmarks.startAccessingSecurityScopedResource(resolvedFile);
    }
  }
}
