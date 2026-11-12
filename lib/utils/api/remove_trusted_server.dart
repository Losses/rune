import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<bool> removeTrustedServer(String fingerprint) async {
  final removeTrustRequest = RemoveTrustedServerRequest(
    fingerprint: fingerprint,
  );

  final rustSignal = await awaitResponse(
    'RemoveTrustedServerRequest',
    RemoveTrustedServerResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) =>
        removeTrustRequest.copyWith(callId: () => callId).sendSignalToRust(),
  );
  final response = rustSignal.message;

  if (!response.success) {
    throw response.error;
  }

  return response.success;
}
