import '../../bindings/bindings.dart';
import 'request_failure.dart';

Future<void> _getQueue = Future.value();

Future<GetDeviceGrantsResponse> getDeviceGrants(String fingerprint) {
  final result = _getQueue.then((_) => _getDeviceGrants(fingerprint));
  _getQueue = result.then<void>((_) {}, onError: (Object _) {});
  return result;
}

Future<GetDeviceGrantsResponse> _getDeviceGrants(String fingerprint) async {
  final signal = await awaitResponse(
    'GetDeviceGrantsRequest',
    GetDeviceGrantsResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) => GetDeviceGrantsRequest(
      fingerprint: fingerprint,
      callId: callId,
    ).sendSignalToRust(),
  );
  return signal.message;
}

Future<void> setDeviceGrants(SetDeviceGrantsRequest request) async {
  await awaitResponse(
    'SetDeviceGrantsRequest',
    SetDeviceGrantsResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) => request.copyWith(callId: () => callId).sendSignalToRust(),
  );
}
