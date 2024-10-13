import 'dart:io';

import 'package:get_storage/get_storage.dart';
import 'package:macos_secure_bookmarks/macos_secure_bookmarks.dart';

class MacSecureManager {
  static final MacSecureManager shared = MacSecureManager();
  static const String storageName = 'mac_secure_manager';

  final GetStorage _storage = GetStorage(storageName);

  static isApplePlatform() {
    return Platform.isMacOS || Platform.isIOS;
  }

  Future<void> saveBookmark(String dir) async {
    if (!isApplePlatform()) {
      return;
    }
    final secureBookmarks = SecureBookmarks();
    final bookmark = await secureBookmarks.bookmark(Directory(dir));
    await _storage.write(dir, bookmark);
  }

  Future<void> loadBookmark() async {
    if (!isApplePlatform()) {
      return;
    }
    final secureBookmarks = SecureBookmarks();

    final bookmarks = _storage.getValues<Iterable<dynamic>>().toList();

    for (final bookmark in bookmarks) {
      final resolvedFile =
          await secureBookmarks.resolveBookmark(bookmark, isDirectory: true);
      await secureBookmarks.startAccessingSecurityScopedResource(resolvedFile);
    }
  }
}
