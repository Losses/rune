import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<void> removeTrustedClient(String fingerprint) async {
  final removeTrustRequest = RemoveTrustedClientRequest(
    fingerprint: fingerprint,
  );

  await awaitResponse(
    'RemoveTrustedClientRequest',
    RemoveTrustedClientResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) =>
        removeTrustRequest.copyWith(callId: () => callId).sendSignalToRust(),
  );
}
