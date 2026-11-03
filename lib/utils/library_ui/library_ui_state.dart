import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../remote_connection_messages.dart';

enum PeerLibrariesStatus { loading, error, empty, rows }

class PeerLibrariesState {
  final PeerLibrariesStatus status;
  final String? error;
  final List<PeerLibrary> libraries;
  const PeerLibrariesState(
    this.status, {
    this.error,
    this.libraries = const [],
  });
}

PeerLibrariesState peerLibrariesState(
  AppLocalizations s,
  ListPeerLibrariesResponse? response,
) {
  if (response == null) {
    return const PeerLibrariesState(PeerLibrariesStatus.loading);
  }
  if (response.errorKind != null) {
    return PeerLibrariesState(
      PeerLibrariesStatus.error,
      error: remoteOpenErrorMessage(s, response.errorKind),
    );
  }
  if (response.libraries.isEmpty) {
    return const PeerLibrariesState(PeerLibrariesStatus.empty);
  }
  return PeerLibrariesState(
    PeerLibrariesStatus.rows,
    libraries: response.libraries,
  );
}

class LibraryRowState {
  final LocalLibrary? library;
  final String subtitle;
  const LibraryRowState(this.library, this.subtitle);
}

LibraryRowState libraryRowState(
  String path,
  List<LocalLibrary> libraries,
  Map<String, String> deviceAliases,
  String Function(String) fromDevice,
) {
  for (final library in libraries) {
    if (library.path == path) {
      final owner = library.ownerFingerprint;
      return LibraryRowState(
        library,
        owner == null || owner.isEmpty
            ? path
            : fromDevice(deviceAliases[owner] ?? library.alias),
      );
    }
  }
  return LibraryRowState(null, path);
}
