import 'package:fluent_ui/fluent_ui.dart';

import '../utils/query_list.dart';
import '../utils/api/play_mode.dart';
import '../utils/api/load_request.dart';
import '../utils/api/operate_playback_with_mix_query.dart';
import '../utils/api/set_media_library_path.dart';
import '../utils/router/navigation.dart';
import '../utils/file_storage/file_storage_service.dart';
import '../utils/settings_manager.dart';

import '../messages/all.dart';
import 'status.dart';

final FileStorageService _fileStorageService = FileStorageService();

Future<String?> getInitialPath() async {
  const String libraryPath =
      String.fromEnvironment('LIBRARY_PATH', defaultValue: "");
  if (libraryPath.isNotEmpty) {
    return libraryPath;
  } else {
    return await _fileStorageService.getLastOpenedFile();
  }
}

class LibraryPathProvider with ChangeNotifier {
  String? _currentPath;
  Set<String> libraryHistory = {};

  LibraryPathProvider(String? initialPath) {
    if (initialPath != null) {
      setLibraryPath(initialPath).then((result) {
        final success = result.$1;
        if (!success) {
          $$replace("/");
        }
      });
    }

    getAllOpenedFiles().then((x) {
      for (final item in x) {
        libraryHistory.add(item);
      }

      if (libraryHistory.isNotEmpty) {
        notifyListeners();
      }
    });
  }

  String? get currentPath => _currentPath;

  Future<(bool, String?)> setLibraryPath(String filePath) async {
    _currentPath = filePath;
    libraryHistory.add(filePath);
    notifyListeners();

    final (success, error) = await setMediaLibraryPath(filePath);

    if (success) {
      _fileStorageService.storeFilePath(filePath);

      await operatePlaybackWithMixQuery(
        queries: const QueryList([("lib::queue", "true")]),
        playbackMode:
            await SettingsManager().getValue<int>(playbackModeKey) ?? 99,
        hintPosition: -1,
        initialPlaybackId: 0,
        instantlyPlay: false,
        operateMode: PlaylistOperateMode.Replace,
        fallbackFileIds: [],
      );

      final lastQueueIndex =
          await (SettingsManager().getValue(lastQueueIndexKey));
      load(lastQueueIndex ?? 0);
    } else {
      removeCurrentPath();
    }

    return (success, error);
  }

  Future<List<String>> getAllOpenedFiles() {
    return _fileStorageService.getAllOpenedFiles();
  }

  // Clear all opened files
  Future<void> clearAllOpenedFiles() async {
    await _fileStorageService.clearAllOpenedFiles();
    _currentPath = null;
    libraryHistory.clear();

    notifyListeners();
  }

  // Add a method to remove a specific file path
  Future<void> removeOpenedFile(String filePath) async {
    await _fileStorageService.removeFilePath(filePath);
    if (_currentPath == filePath) {
      _currentPath = null;
      libraryHistory.remove(filePath);
    }
    notifyListeners();
  }

  removeCurrentPath() {
    _currentPath = null;
    notifyListeners();
  }
}
