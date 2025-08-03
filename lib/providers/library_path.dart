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
import '../bindings/bindings.dart';
import '../constants/configurations.dart';

final FileStorageService _fileStorageService = FileStorageService();

enum ConnectionType {
  local,
  remote,
}

@immutable
class LibraryPathEntry {
  final String rawPath;
  final String cleanPath;
  final OperationDestination source;
  final OperationDestination destination;
  final String alias;

  const LibraryPathEntry._({
    required this.rawPath,
    required this.cleanPath,
    required this.source,
    required this.destination,
    required this.alias,
  });

  factory LibraryPathEntry(String path, {String? alias}) {
    final (src, dest) = determineConnectionType(path);
    return LibraryPathEntry._(
      rawPath: path,
      cleanPath: removePrefix(path),
      source: src,
      destination: dest,
      alias: alias ?? _generateDefaultAlias(removePrefix(path), src, dest),
    );
  }

  factory LibraryPathEntry.fromLegacy(String path) {
    return LibraryPathEntry(path);
  }

  static String _generateDefaultAlias(
    String cleanPath,
    OperationDestination src,
    OperationDestination dest,
  ) {
    if (src == OperationDestination.local &&
        dest == OperationDestination.local) {
      final segments = cleanPath.split('/');
      return segments.lastWhere((s) => s.isNotEmpty, orElse: () => cleanPath);
    }

    final uri = Uri.tryParse(cleanPath);
    if (uri != null && uri.host.isNotEmpty) {
      return uri.host;
    }
    return cleanPath;
  }

  static String removePrefix(String? path) {
    if (path == null) return "";
    if (path.startsWith('@RR|')) return path.substring(4);
    if (path.startsWith('@LR|')) return path.substring(4);
    return path;
  }

  @override
  bool operator ==(Object other) =>
      identical(this, other) ||
      other is LibraryPathEntry &&
          runtimeType == other.runtimeType &&
          rawPath == other.rawPath;

  @override
  int get hashCode => rawPath.hashCode;

  @override
  String toString() => '[$source→$destination] $cleanPath';
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
  final Set<LibraryPathEntry> _libraryHistory = {};

  LibraryPathProvider(String? initialPath) {
    if (initialPath != null) {
      setLibraryPath(null, initialPath, null).then((result) {
        final success = result.$1;
        if (!success) {
          $$replace("/");
        }
      });
    }

    _fileStorageService.getAllOpenedFiles().then((files) {
      _libraryHistory.addAll(files.map((p) => LibraryPathEntry(p)));

      if (_libraryHistory.isNotEmpty) {
        notifyListeners();
      }
    });
  }

  String? get currentPath => _currentPath;

  Set<LibraryPathEntry> get libraryHistory => _libraryHistory;

  void addLibraryPath(
      BuildContext? context, String filePath, String alias) async {
    final entry = LibraryPathEntry(filePath, alias: alias);

    if (_libraryHistory.add(entry)) {
      notifyListeners();
    }

    _fileStorageService.storeFilePath(filePath, alias: alias);
  }

  Future<(bool, bool, String?)> setLibraryPath(
    BuildContext? context,
    String filePath,
    LibraryInitializeMode? selectedMode,
  ) async {
    _currentPath = filePath;
    notifyListeners();

    var (success, notReady, error) =
        await setMediaLibraryPath(filePath, selectedMode);

    if (!success) {
      return (false, false, error);
    }

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
      final entry = LibraryPathEntry(filePath);
      if (_libraryHistory.add(entry)) {
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
        operateMode: PlaylistOperateMode.replace,
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

  List<LibraryPathEntry> _filterEntries({
    OperationDestination? source,
    OperationDestination? destination,
  }) {
    return _libraryHistory.where((entry) {
      final sourceMatch = source == null || entry.source == source;
      final destMatch = destination == null || entry.destination == destination;
      return sourceMatch && destMatch;
    }).toList();
  }

  List<LibraryPathEntry> getRRPaths() => _filterEntries(
        source: OperationDestination.remote,
        destination: OperationDestination.remote,
      );

  List<LibraryPathEntry> getLLPaths() => _filterEntries(
        source: OperationDestination.local,
        destination: OperationDestination.local,
      );

  List<LibraryPathEntry> getAnySourceRemotePaths() =>
      _filterEntries(destination: OperationDestination.remote);

  List<LibraryPathEntry> getAnyDestinationRemotePaths() =>
      _filterEntries(source: OperationDestination.remote);

  Future<void> clearAllOpenedFiles() async {
    await _fileStorageService.clearAllOpenedFiles();
    _currentPath = null;
    _libraryHistory.clear();
    notifyListeners();
  }

  Future<void> removeOpenedFile(String filePath) async {
    await _fileStorageService.removeFilePath(filePath);

    final originalCount = _libraryHistory.length;

    _libraryHistory.removeWhere((entry) => entry.rawPath == filePath);

    bool hasChanged = _libraryHistory.length != originalCount;

    if (_currentPath == filePath) {
      _currentPath = null;
      hasChanged = true;
    }

    if (hasChanged) {
      notifyListeners();
    }
  }

  Future<void> removeOpenedFileByCleanPath(String cleanPath) async {
    final toRemove =
        _libraryHistory.where((e) => e.cleanPath == cleanPath).toList();

    for (final entry in toRemove) {
      await _fileStorageService.removeFilePath(entry.rawPath);
      _libraryHistory.remove(entry);
    }
  }

  void removeCurrentPath() {
    _currentPath = null;
    notifyListeners();
  }
}
