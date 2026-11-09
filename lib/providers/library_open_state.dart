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
  LibraryOpenStateProvider(String? initialPath, this.registry) {
    registry.addListener(_registryChanged);
    initialOpen = _initializeRegistryLibrary(initialPath);
  }

  final LibraryRegistryProvider registry;
  late final Future<bool> initialOpen;

  LibraryRecordSummary? get openRecord => registry.openRecord;
  bool get isRemoteControl =>
      openRecord?.kind is LibraryRecordKindSummaryRemoteControl;
  bool get isReplica => openRecord?.kind is LibraryRecordKindSummaryReplica;
  String? get catalogLibraryId => switch (openRecord?.kind) {
    LibraryRecordKindSummaryLocal(:final libraryId) => libraryId,
    LibraryRecordKindSummaryReplica(:final libraryId) => libraryId,
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
      } else if (registry.lastOpenRecordId != null) {
        await registry.open(registry.lastOpenRecordId!);
        return true;
      }
    } on Object catch (error) {
      error$('Could not restore the last library: $error');
    }
    return false;
  }

  void _registryChanged() {
    notifyListeners();
  }

  @override
  void dispose() {
    registry.removeListener(_registryChanged);
    super.dispose();
  }
}
