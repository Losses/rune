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
    send: () =>
        GetDeviceGrantsRequest(fingerprint: fingerprint).sendSignalToRust(),
  );
  return signal.message;
}

Future<void> setDeviceGrants(SetDeviceGrantsRequest request) async {
  final signal = await awaitResponse(
    'SetDeviceGrantsRequest',
    SetDeviceGrantsResponse.rustSignalStream,
    send: request.sendSignalToRust,
  );
  final response = signal.message;
  if (response.errorKind != null || response.error != null) {
    throw RequestFailure(
      'SetDeviceGrantsRequest',
      response.errorKind ?? RequestErrorKind.internal,
      response.error ?? '',
    );
  }
}
