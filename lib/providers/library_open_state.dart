import 'package:fluent_ui/fluent_ui.dart';

import '../utils/rune_log.dart';
import 'library_registry.dart';
import '../bindings/bindings.dart';

Future<String?> getInitialPath() async {
  const libraryPath = String.fromEnvironment('LIBRARY_PATH', defaultValue: '');
  return libraryPath.isEmpty ? null : libraryPath;
}

bool shouldOpenLibraryAtStartup(
  String? initialPath,
  LibraryRegistryProvider registry,
) {
  final lastOpenRecordId = registry.lastOpenRecordId;
  return initialPath != null ||
      (lastOpenRecordId != null &&
          !registry.invalidRecords.any(
            (record) => record.recordId == lastOpenRecordId,
          ));
}

class LibraryOpenStateProvider with ChangeNotifier {
  LibraryOpenStateProvider(
    String? initialPath,
    this.registry, {
    Future<void> Function(String recordId)? beforeOpen,
  }) : _beforeOpen = beforeOpen {
    registry.addListener(_registryChanged);
    initialOpen = _initializeRegistryLibrary(initialPath).whenComplete(() {
      _openingAtStartup = false;
      if (!_disposed) notifyListeners();
    });
  }

  final LibraryRegistryProvider registry;

  /// Readies the record open last before startup opens it again, as by signing it in. What it
  /// throws keeps the library closed, as a failed open does.
  final Future<void> Function(String recordId)? _beforeOpen;
  late final Future<bool> initialOpen;

  /// Why startup did not open a library again: a request that failed, or what readying the
  /// record threw.
  Object? initialOpenFailure;
  bool _openingAtStartup = true;
  bool _disposed = false;

  /// Whether startup is still opening a library again, as while it waits for a sign-in.
  bool get isOpeningAtStartup => _openingAtStartup;

  LibraryRecordSummary? get openRecord => registry.openRecord;
  bool get isRemoteControl =>
      openRecord?.kind is LibraryRecordKindSummaryRemoteControl;
  bool get isReplica => openRecord?.kind is LibraryRecordKindSummaryReplica;
  String? get catalogLibraryId => switch (openRecord?.kind) {
    LibraryRecordKindSummaryLocal(:final libraryId) => libraryId,
    LibraryRecordKindSummaryReplica(:final libraryId) => libraryId,
    LibraryRecordKindSummaryOpenDal(:final libraryId) => libraryId,
    _ => null,
  };
  bool get hasCatalog => catalogLibraryId != null;
  bool get isReadOnlyReplica => switch (openRecord) {
    LibraryRecordSummary(
      kind: LibraryRecordKindSummaryReplica(:final ownerFingerprint),
      syncAccess: LibraryAccessSummary.read,
    ) when ownerFingerprint.isNotEmpty => true,
    _ => false,
  };

  String? get openLocalFolderPath => switch (openRecord?.kind) {
    LibraryRecordKindSummaryLocal(:final path) => path,
    _ => null,
  };

  bool get hasOpenLibrary => openRecord != null;

  Future<bool> _initializeRegistryLibrary(String? initialPath) async {
    await registry.ready;
    try {
      if (initialPath != null && initialPath.isNotEmpty) {
        final segments = initialPath.split('/').where((part) => part.isNotEmpty);
        final alias = segments.isEmpty ? initialPath : segments.last;
        final response = await registry.registerLocalFolder(
          initialPath,
          alias,
          LibraryInitializeMode.portable,
        );
        if (response.record != null) {
          await registry.open(response.record!.recordId);
          return true;
        }
      } else if (registry.lastOpenRecordId case final recordId?) {
        await _beforeOpen?.call(recordId);
        await registry.open(recordId);
        return true;
      }
    } on Object catch (error) {
      initialOpenFailure = error;
      error$('Could not restore the last library: $error');
    }
    return false;
  }

  void _registryChanged() {
    notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    registry.removeListener(_registryChanged);
    super.dispose();
  }
}
