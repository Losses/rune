import 'request_failure.dart';
import '../../bindings/bindings.dart';

const _timeout = Duration(seconds: 30);

Future<ReplaceSubsonicCredentialResponse> replaceSubsonicCredential(
  String recordId,
  String username,
  String password,
) async {
  final response = await awaitResponse(
    'ReplaceSubsonicCredentialRequest',
    ReplaceSubsonicCredentialResponse.rustSignalStream,
    timeout: _timeout,
    send: () => ReplaceSubsonicCredentialRequest(
      recordId: recordId,
      username: username,
      password: password,
    ).sendSignalToRust(),
  );
  return response.message;
}

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

Future<RegisterLibraryResponse> registerOpenDalLibrary(
  OpenDalLibraryService service,
  String alias,
) async {
  final response = awaitResponse(
    'RegisterOpenDalLibraryRequest',
    RegisterLibraryResponse.rustSignalStream,
    timeout: _timeout,
    send: () => RegisterOpenDalLibraryRequest(
      service: service,
      alias: alias,
    ).sendSignalToRust(),
  );
  return (await response).message;
}

Future<RegisterLibraryResponse> registerSubsonicLibrary(
  String endpoint,
  String username,
  String password,
  String alias,
  SubsonicOpenModeMessage? mode,
) async {
  final response = awaitResponse(
    'RegisterSubsonicLibraryRequest',
    RegisterLibraryResponse.rustSignalStream,
    timeout: _timeout,
    send: () => RegisterSubsonicLibraryRequest(
      endpoint: endpoint,
      username: username,
      password: password,
      alias: alias,
      mode: mode,
    ).sendSignalToRust(),
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
      connection: library.connection,
      sync: library.sync,
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
