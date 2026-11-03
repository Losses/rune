import '../../bindings/bindings.dart';

const _timeout = Duration(seconds: 30);

Future<ListPeerLibrariesResponse> listPeerLibraries(String fingerprint) async {
  final response = ListPeerLibrariesResponse.rustSignalStream.first.timeout(
    _timeout,
  );
  ListPeerLibrariesRequest(fingerprint: fingerprint).sendSignalToRust();
  return (await response).message;
}

Future<AddRemoteLibraryResponse> addRemoteLibrary(
  PeerLibrary library,
  String fingerprint,
) async {
  final response = AddRemoteLibraryResponse.rustSignalStream.first.timeout(
    _timeout,
  );
  AddRemoteLibraryRequest(
    fingerprint: fingerprint,
    libraryId: library.libraryId,
    alias: library.alias,
  ).sendSignalToRust();
  return (await response).message;
}

Future<ListLocalLibrariesResponse> listLocalLibraries() async {
  final response = ListLocalLibrariesResponse.rustSignalStream.first.timeout(
    _timeout,
  );
  const ListLocalLibrariesRequest().sendSignalToRust();
  return (await response).message;
}

Future<SetLibraryBackgroundSyncResponse> setLibraryBackgroundSync(
  String libraryId,
  bool enabled,
) async {
  final response = SetLibraryBackgroundSyncResponse.rustSignalStream.first
      .timeout(_timeout);
  SetLibraryBackgroundSyncRequest(
    libraryId: libraryId,
    enabled: enabled,
  ).sendSignalToRust();
  return (await response).message;
}
