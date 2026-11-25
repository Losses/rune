import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<void> removeTrustedServer(String fingerprint) async {
  final removeTrustRequest = RemoveTrustedServerRequest(
    fingerprint: fingerprint,
  );

  await awaitResponse(
    'RemoveTrustedServerRequest',
    RemoveTrustedServerResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) =>
        removeTrustRequest.copyWith(callId: () => callId).sendSignalToRust(),
  );
}
