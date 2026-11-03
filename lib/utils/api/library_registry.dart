import 'request_failure.dart';
import '../../bindings/bindings.dart';

const _timeout = Duration(seconds: 30);

Future<ListPeerLibrariesResponse> listPeerLibraries(String fingerprint) async {
  final response = awaitResponse(
    'ListPeerLibrariesRequest',
    ListPeerLibrariesResponse.rustSignalStream,
    timeout: _timeout,
    send: () =>
        ListPeerLibrariesRequest(fingerprint: fingerprint).sendSignalToRust(),
  );
  return (await response).message;
}

Future<AddRemoteLibraryResponse> addRemoteLibrary(
  PeerLibrary library,
  String fingerprint,
) async {
  final response = awaitResponse(
    'AddRemoteLibraryRequest',
    AddRemoteLibraryResponse.rustSignalStream,
    timeout: _timeout,
    send: () => AddRemoteLibraryRequest(
      fingerprint: fingerprint,
      libraryId: library.libraryId,
      alias: library.alias,
    ).sendSignalToRust(),
  );
  return (await response).message;
}

Future<ListLocalLibrariesResponse> listLocalLibraries() async {
  final response = awaitResponse(
    'ListLocalLibrariesRequest',
    ListLocalLibrariesResponse.rustSignalStream,
    timeout: _timeout,
    send: () => const ListLocalLibrariesRequest().sendSignalToRust(),
  );
  return (await response).message;
}

Future<SetLibraryBackgroundSyncResponse> setLibraryBackgroundSync(
  String libraryId,
  bool enabled,
) async {
  final response = awaitResponse(
    'SetLibraryBackgroundSyncRequest',
    SetLibraryBackgroundSyncResponse.rustSignalStream,
    timeout: _timeout,
    send: () => SetLibraryBackgroundSyncRequest(
      libraryId: libraryId,
      enabled: enabled,
    ).sendSignalToRust(),
  );
  return (await response).message;
}
