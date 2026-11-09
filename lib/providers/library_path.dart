import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';

import '../utils/api/library_registry.dart';
import '../utils/library_ui/library_ui_state.dart';
import '../utils/rune_log.dart';
import '../utils/api/request_failure.dart';
import '../utils/api/set_media_library_path.dart';
import 'library_registry.dart';
import '../bindings/bindings.dart';

enum ConnectionType { local, remote }

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
  const String libraryPath = String.fromEnvironment(
    'LIBRARY_PATH',
    defaultValue: "",
  );
  if (libraryPath.isNotEmpty) {
    return libraryPath;
  } else {
    return null;
  }
}

class LibraryPathProvider with ChangeNotifier {
  LibraryPathProvider(String? initialPath, [this.registry]) {
    registry?.addListener(_registryChanged);
    _contentSubscription = LibraryContentChanged.rustSignalStream.listen((_) {
      _refreshAccess();
    });
    initialOpen = registry == null
        ? Future.value(false)
        : _initializeRegistryLibrary(initialPath);
  }

  final LibraryRegistryProvider? registry;
  StreamSubscription<dynamic>? _contentSubscription;
  bool _readOnly = false;
  int _refreshVersion = 0;
  bool _disposed = false;
  late final Future<bool> initialOpen;

  bool get isReadOnlyReplica => _readOnly;

  // L1c3: remove
  String? get currentPath {
    final currentRegistry = registry;
    final id = currentRegistry?.openRecordId;
    if (id == null || currentRegistry == null) return null;
    for (final record in currentRegistry.records) {
      if (record.recordId != id) continue;
      return switch (record.kind) {
        LibraryRecordKindSummaryLocal(:final path) => path,
        LibraryRecordKindSummaryReplica(:final path) => path,
        LibraryRecordKindSummaryRemoteControl(
          :final fingerprint,
          :final endpoint,
        ) =>
          endpoint ?? '@RR|$fingerprint',
        LibraryRecordKindSummarySubsonic(:final endpoint) => endpoint,
        _ => null,
      };
    }
    return null;
  }

  Future<bool> _initializeRegistryLibrary(String? initialPath) async {
    final currentRegistry = registry!;
    await currentRegistry.ready;
    try {
      if (initialPath != null && initialPath.isNotEmpty) {
        final segments = initialPath
            .split('/')
            .where((part) => part.isNotEmpty);
        final alias = segments.isEmpty ? initialPath : segments.last;
        final response = await currentRegistry.registerLocalFolder(
          initialPath,
          alias,
          LibraryInitializeMode.portable,
        );
        if (response.record != null) {
          await currentRegistry.open(response.record!.recordId);
          return true;
        }
      } else if (currentRegistry.lastOpenRecordId != null) {
        await currentRegistry.open(currentRegistry.lastOpenRecordId!);
        return true;
      }
    } on Object catch (error) {
      error$('Could not restore the last library: $error');
    }
    return false;
  }

  void _registryChanged() {
    _refreshAccess();
    notifyListeners();
  }

  Future<void> _refreshAccess() async {
    final version = ++_refreshVersion;
    final path = currentPath;
    if (path == null || path.startsWith('@RR|')) {
      if (_readOnly) {
        _readOnly = false;
        notifyListeners();
      }
      return;
    }
    try {
      final libraries = await listLocalLibraries();
      if (_disposed || version != _refreshVersion) return;
      final readOnly = currentLibraryIsReadOnly(path, libraries.libraries);
      if (readOnly != _readOnly) {
        _readOnly = readOnly;
        notifyListeners();
      }
    } on RequestFailure catch (error) {
      error$(error.toString());
    }
  }

  void removeCurrentPath() {
    registry?.onClosed();
    _readOnly = false;
    _refreshVersion++;
    notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    _contentSubscription?.cancel();
    registry?.removeListener(_registryChanged);
    super.dispose();
  }
}
