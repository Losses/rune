import 'package:flutter/foundation.dart';

import '../../utils/rune_log.dart';
import '../../utils/settings_manager.dart';
import '../../utils/file_storage/mac_secure_manager.dart';
import '../../providers/library_path.dart';
import '../../constants/configurations.dart';

final SettingsManager settingsManager = SettingsManager();

class FileStorageService {
  static const _currentDataVersion = 2;

  // Get the list of opened files
  Future<List<Map<String, dynamic>>> _getOpenedFiles() async {
    final version = await settingsManager.getValue<int>(kDataVersionKey) ?? 1;
    if (version < _currentDataVersion) {
      await _migrateData(version);
    }

    final raw = await settingsManager.getValue<List<dynamic>>(kOpenedFilesKey);

    return normalizeOpenedFiles(raw ?? []);
  }

  // Older versions of Rune store bare paths and do not touch the data version,
  // so check every entry instead of trusting the version.
  @visibleForTesting
  List<Map<String, dynamic>> normalizeOpenedFiles(List<dynamic> raw) {
    final entries = <Map<String, dynamic>>[];
    for (final item in raw) {
      if (item is String && item.isNotEmpty) {
        entries.add(_migrateLegacyPath(item));
      } else if (item is Map<String, dynamic> &&
          item['path'] is String &&
          item['path'] != '') {
        entries.add(Map<String, dynamic>.from(item));
      } else {
        warn$("Ignoring opened library entry: $item");
      }
    }
    return entries;
  }

  Future<void> _migrateData(int oldVersion) async {
    if (oldVersion == 1) {
      final raw =
          await settingsManager.getValue<List<dynamic>>(kOpenedFilesKey);

      await settingsManager.setValue(
          kOpenedFilesKey, normalizeOpenedFiles(raw ?? []));
      await settingsManager.setValue(kDataVersionKey, _currentDataVersion);
    }
  }

  Map<String, dynamic> _migrateLegacyPath(String path) {
    final entry = LibraryPathEntry(path);
    return {
      'path': path,
      'alias': entry.alias,
      'createdAt': DateTime.now().toIso8601String(),
    };
  }

  // Store file path
  Future<void> storeFilePath(String path, {String? alias}) async {
    final entries = await _getOpenedFiles();

    entries.removeWhere((e) => e['path'] == path);

    final entry = LibraryPathEntry(path, alias: alias);
    entries.add({
      'path': path,
      'alias': entry.alias,
      'createdAt': DateTime.now().toIso8601String(),
    });

    await MacSecureManager().saveBookmark(path);
    await settingsManager.setValue(kOpenedFilesKey, entries);
  }

  Future<void> renameFilePath(String oldPath, String newAlias) async {
    final entries = await _getOpenedFiles();
    final index = entries.indexWhere((e) => e['path'] == oldPath);

    if (index != -1) {
      entries[index]['alias'] = newAlias;
      await settingsManager.setValue(kOpenedFilesKey, entries);
    }
  }

  // Get the last opened file
  Future<String?> getLastOpenedFile() async {
    final entries = await _getOpenedFiles();
    if (entries.isNotEmpty) {
      return entries.last['path'] as String?;
    }
    return null;
  }

  // Get all opened files
  Future<List<String>> getAllOpenedFiles() async {
    final entries = await _getOpenedFiles();
    return entries.map((e) => e['path'] as String).toList();
  }

  // Clear all opened files
  Future<void> clearAllOpenedFiles() async {
    await settingsManager.removeValue(kOpenedFilesKey);
  }

  // Remove a specific file path
  Future<void> removeFilePath(String filePath) async {
    final entries = await _getOpenedFiles();
    final newEntries = entries.where((e) => e['path'] != filePath).toList();
    await settingsManager.setValue(kOpenedFilesKey, newEntries);
  }
}
