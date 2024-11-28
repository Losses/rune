import 'package:fluent_ui/fluent_ui.dart';

import '../utils/query_list.dart';
import '../utils/api/play_mode.dart';
import '../utils/api/load_request.dart';
import '../utils/api/set_media_library_path.dart';
import '../utils/api/operate_playback_with_mix_query.dart';
import '../utils/settings_manager.dart';
import '../utils/router/navigation.dart';
import '../utils/dialogs/select_library_mode/show_select_library_mode_dialog.dart';
import '../utils/file_storage/file_storage_service.dart';
import '../screens/collection/utils/collection_data_provider.dart';
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
      setLibraryPath(null, initialPath, null).then((result) {
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

  Future<(bool, bool, String?)> setLibraryPath(
    BuildContext? context,
    String filePath,
    LibraryInitializeMode? selectedMode,
  ) async {
    _currentPath = filePath;
    libraryHistory.add(filePath);
    notifyListeners();

    var (success, notReady, error) =
        await setMediaLibraryPath(filePath, selectedMode);

    print('S$success, NR$notReady, E$error');

    if (notReady && context == null) {
      print('OOH!');
      return (false, true, null);
    }

    if (notReady && context != null) {
      print('!!!!');
      selectedMode = stringToLibraryInitializeMode(
        await showSelectLibraryModeDialog(context),
      );

      if (selectedMode == null) {
        return (false, true, null);
      }

      (success, notReady, error) =
          await setMediaLibraryPath(filePath, selectedMode);
    }

    if (success) {
      CollectionCache().clearAll();
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

    return (success, false, error);
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
