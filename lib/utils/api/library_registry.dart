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
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) => ReplaceSubsonicCredentialRequest(
      recordId: recordId,
      username: username,
      password: password,
      callId: callId,
    ).sendSignalToRust(),
  );
  return response.message;
}

Future<ListPeerLibrariesResponse> listPeerLibraries(String fingerprint) async {
  final response = awaitResponse(
    'ListPeerLibrariesRequest',
    ListPeerLibrariesResponse.rustSignalStream,
    timeout: _timeout,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) => ListPeerLibrariesRequest(
      fingerprint: fingerprint,
      callId: callId,
    ).sendSignalToRust(),
  );
  return (await response).message;
}

Future<RegisterLibraryResponse> registerOpenDalLibrary(
  OpenDalLibraryService service,
  String alias,
  ShareDataChoice shareDataChoice,
) async {
  final response = awaitResponse(
    'RegisterOpenDalLibraryRequest',
    RegisterLibraryResponse.rustSignalStream,
    timeout: _timeout,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) => RegisterOpenDalLibraryRequest(
      service: service,
      alias: alias,
      shareDataChoice: shareDataChoice,
      callId: callId,
    ).sendSignalToRust(),
  );
  return (await response).message;
}

Future<DetectOpenDalShareDataResponse> detectOpenDalShareData(
  OpenDalLibraryService service,
) async {
  final response = await awaitResponse(
    'DetectOpenDalShareDataRequest',
    DetectOpenDalShareDataResponse.rustSignalStream,
    timeout: _timeout,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) => DetectOpenDalShareDataRequest(
      service: service,
      callId: callId,
    ).sendSignalToRust(),
  );
  return response.message;
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
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) => RegisterSubsonicLibraryRequest(
      endpoint: endpoint,
      username: username,
      password: password,
      alias: alias,
      mode: mode,
      callId: callId,
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
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) => AddRemoteLibraryRequest(
      fingerprint: fingerprint,
      libraryId: library.libraryId,
      alias: library.alias,
      connection: library.connection,
      sync: library.sync,
      callId: callId,
    ).sendSignalToRust(),
  );
  return (await response).message;
}

Future<ListLocalLibrariesResponse> listLocalLibraries() async {
  final response = awaitResponse(
    'ListLocalLibrariesRequest',
    ListLocalLibrariesResponse.rustSignalStream,
    timeout: _timeout,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) =>
        ListLocalLibrariesRequest(callId: callId).sendSignalToRust(),
  );
  return (await response).message;
}

Future<void> setLibraryBackgroundSync(String libraryId, bool enabled) async {
  await awaitResponse(
    'SetLibraryBackgroundSyncRequest',
    SetLibraryBackgroundSyncResponse.rustSignalStream,
    timeout: _timeout,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) => SetLibraryBackgroundSyncRequest(
      libraryId: libraryId,
      enabled: enabled,
      callId: callId,
    ).sendSignalToRust(),
  );
}
