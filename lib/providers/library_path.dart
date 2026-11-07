import 'dart:async';
import 'dart:io';

import 'package:fluent_ui/fluent_ui.dart';

import '../utils/query_list.dart';
import '../utils/api/library_registry.dart';
import '../utils/library_ui/library_ui_state.dart';
import '../utils/rune_log.dart';
import '../utils/api/load_request.dart';
import '../utils/api/request_failure.dart';
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
  bool _openedLocalLibrary = false;
  String? _currentPath;
  final Set<LibraryPathEntry> _libraryHistory = {};
  StreamSubscription<dynamic>? _contentSubscription;
  bool _readOnly = false;
  int _refreshVersion = 0;
  bool _disposed = false;
  late final Future<bool> initialOpen;

  bool get isReadOnlyReplica => _readOnly;

  Future<void> _refreshAccess() async {
    final version = ++_refreshVersion;
    if (_currentPath == null || _currentPath!.startsWith('@RR|')) return;
    try {
      final libraries = await listLocalLibraries();
      if (_disposed || version != _refreshVersion) return;
      final readOnly =
          currentLibraryIsReadOnly(_currentPath, libraries.libraries);
      if (readOnly != _readOnly) {
        _readOnly = readOnly;
        notifyListeners();
      }
    } on RequestFailure catch (error) {
      error$(error.toString());
    }
  }

  @override
  void dispose() {
    _disposed = true;
    _contentSubscription?.cancel();
    super.dispose();
  }

  LibraryPathProvider(String? initialPath) {
    _contentSubscription = LibraryContentChanged.rustSignalStream.listen((_) {
      _refreshAccess();
    });
    if (initialPath != null) {
      initialOpen = setLibraryPath(null, initialPath, null).then((result) {
        final success = result.$1;
        if (!success) $$replace("/");
        return success;
      });
    } else {
      initialOpen = Future.value(false);
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

  Future<void> addLibraryPath(
      BuildContext? context, String filePath, String alias) async {
    final entry = LibraryPathEntry(filePath, alias: alias);

    if (_libraryHistory.add(entry)) {
      notifyListeners();
    }

    await _fileStorageService.storeFilePath(filePath, alias: alias);
  }

  Future<(bool, bool, String?, LibraryOpenErrorKind?)> setLibraryPath(
    BuildContext? context,
    String filePath,
    LibraryInitializeMode? selectedMode,
  ) async {
    _currentPath = filePath;
    _readOnly = false;
    _refreshAccess();
    notifyListeners();

    var restoreLastIndex = false;
    Future<(bool, bool, String?, LibraryOpenErrorKind?)> openLibrary() async {
      final subscription = SetMediaLibraryPathResponse.rustSignalStream
          .where(
            (event) =>
                event.message.path == LibraryPathEntry.removePrefix(filePath),
          )
          .listen((event) {
            restoreLastIndex = event.message.restoreLastIndex;
          });
      try {
        return await setMediaLibraryPath(filePath, selectedMode);
      } finally {
        await subscription.cancel();
      }
    }

    (bool, bool, String?, LibraryOpenErrorKind?) result;
    try {
      result = await openLibrary();
    } on RequestFailure {
      removeCurrentPath();
      rethrow;
    }
    var (success, notReady, error, errorKind) = result;

    if (!success) {
      if (error != null) error$(error);
      removeCurrentPath();
      return (false, false, error, errorKind);
    }

    if (notReady && context == null) {
      return (false, true, null, null);
    }

    if (notReady && context != null) {
      if (!context.mounted) return (false, true, null, null);
      // Databases cannot live inside a SAF tree (SQLite WAL needs real
      // file paths), so Android is always redirected to app-local storage.
      if (Platform.isAndroid) {
        selectedMode = LibraryInitializeMode.redirected;
      } else {
        selectedMode = stringToLibraryInitializeMode(
          await showSelectLibraryModeDialog(context),
        );
      }

      if (selectedMode == null) {
        return (false, true, null, null);
      }

      try {
        (success, notReady, error, errorKind) = await openLibrary();
      } on RequestFailure {
        removeCurrentPath();
        rethrow;
      }
    }

    if (success) {
      await _refreshAccess();
      final entry = LibraryPathEntry(filePath);
      if (_libraryHistory.add(entry)) {
        notifyListeners();
      }
      CollectionCache().clearAll();
      _fileStorageService.storeFilePath(filePath);

      try {
        if (filePath.startsWith('@RR|')) {
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

          final lastQueueIndex = await (SettingsManager().getValue(
            kLastQueueIndexKey,
          ));
          load(lastQueueIndex ?? 0);
        } else {
          if (!_openedLocalLibrary && restoreLastIndex) {
            final mode = await SettingsManager().getValue<int>(
              kPlaybackModeKey,
            );
            if (mode != null && mode != 99) {
              SetPlaybackModeRequest(mode: mode).sendSignalToRust();
            }
            final index = await SettingsManager().getValue<int>(
              kLastQueueIndexKey,
            );
            load(index ?? 0);
          }
          _openedLocalLibrary = true;
        }
      } on RequestFailure {
        // Opening the library does not depend on restoring its queue.
      }
    } else {
      removeCurrentPath();
    }

    if (!success && error != null) error$(error);
    return (success, false, error, errorKind);
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
    _readOnly = false;
    _refreshVersion++;
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
      _readOnly = false;
      _refreshVersion++;
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
    _readOnly = false;
    _refreshVersion++;
    notifyListeners();
  }
}
