import 'package:fluent_ui/fluent_ui.dart';

import '../utils/query_list.dart';
import '../utils/api/load_request.dart';
import '../utils/api/set_media_library_path.dart';
import '../utils/api/operate_playback_with_mix_query.dart';
import '../utils/settings_manager.dart';
import '../utils/router/navigation.dart';
import '../utils/dialogs/select_library_mode/show_select_library_mode_dialog.dart';
import '../utils/file_storage/file_storage_service.dart';
import '../screens/collection/utils/collection_data_provider.dart';
import '../messages/all.dart';
import '../constants/configurations.dart';

final FileStorageService _fileStorageService = FileStorageService();

enum ConnectionType {
  local,
  remote,
}

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
  final Map<String, (OperationDestination, OperationDestination)>
      _libraryHistory = {};

  LibraryPathProvider(String? initialPath) {
    if (initialPath != null) {
      setLibraryPath(null, initialPath, null).then((result) {
        final success = result.$1;
        if (!success) {
          $$replace("/");
        }
      });
    }

    getAllOpenedFiles().then((paths) {
      for (final path in paths) {
        final connectionType = determineConnectionType(path);
        _libraryHistory[path] = connectionType;
      }
      if (_libraryHistory.isNotEmpty) {
        notifyListeners();
      }
    });
  }

  String? get currentPath => _currentPath;

  Map<String, (OperationDestination, OperationDestination)>
      get libraryHistory => _libraryHistory;

  Future<(bool, bool, String?)> setLibraryPath(
    BuildContext? context,
    String filePath,
    LibraryInitializeMode? selectedMode,
  ) async {
    _currentPath = filePath;
    notifyListeners();

    var (success, notReady, error) =
        await setMediaLibraryPath(filePath, selectedMode);

    if (notReady && context == null) {
      return (false, true, null);
    }

    if (notReady && context != null) {
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
      final connectionType = determineConnectionType(filePath);
      if (!_libraryHistory.containsKey(filePath)) {
        _libraryHistory[filePath] = connectionType;
        notifyListeners();
      }
      CollectionCache().clearAll();
      _fileStorageService.storeFilePath(filePath);

      await operatePlaybackWithMixQuery(
        queries: const QueryList([("lib::queue", "true")]),
        playbackMode:
            await SettingsManager().getValue<int>(kPlaybackModeKey) ?? 99,
        hintPosition: -1,
        initialPlaybackId: 0,
        instantlyPlay: false,
        operateMode: PlaylistOperateMode.Replace,
        fallbackPlayingItems: [],
      );

      final lastQueueIndex =
          await (SettingsManager().getValue(kLastQueueIndexKey));
      load(lastQueueIndex ?? 0);
    } else {
      removeCurrentPath();
    }

    return (success, false, error);
  }

  List<String> _getPaths({
    OperationDestination? source,
    OperationDestination? destination,
  }) {
    return _libraryHistory.entries
        .where((entry) {
          final (src, dest) = entry.value;

          final sourceMatch = source == null || src == source;
          final destMatch = destination == null || dest == destination;

          return sourceMatch && destMatch;
        })
        .map((entry) => entry.key)
        .toList();
  }

  List<String> getDestinationRemotePaths() =>
      _getPaths(destination: OperationDestination.Remote);

  List<String> getSourceRemotePaths() =>
      _getPaths(source: OperationDestination.Remote);

  List<String> getRRPaths() => _getPaths(
        source: OperationDestination.Remote,
        destination: OperationDestination.Remote,
      );

  List<String> getLRPaths() => _getPaths(
        source: OperationDestination.Local,
        destination: OperationDestination.Remote,
      );

  List<String> getLLPaths() => _getPaths(
        source: OperationDestination.Local,
        destination: OperationDestination.Local,
      );

  Future<List<String>> getAllOpenedFiles() {
    return _fileStorageService.getAllOpenedFiles();
  }

  Future<void> clearAllOpenedFiles() async {
    await _fileStorageService.clearAllOpenedFiles();
    _currentPath = null;
    _libraryHistory.clear();
    notifyListeners();
  }

  Future<void> removeOpenedFile(String filePath) async {
    await _fileStorageService.removeFilePath(filePath);
    if (_currentPath == filePath) {
      _currentPath = null;
    }
    _libraryHistory.remove(filePath);
    notifyListeners();
  }

  void removeCurrentPath() {
    _currentPath = null;
    notifyListeners();
  }
}
