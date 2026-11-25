import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<void> addTrustedServer(String fingerprint, List<String> hosts) async {
  final removeTrustRequest = AddTrustedServerRequest(
    certificate: TrustedServerCertificate(
      fingerprint: fingerprint,
      hosts: hosts,
    ),
  );

  await awaitResponse(
    'AddTrustedServerRequest',
    AddTrustedServerResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) =>
        removeTrustRequest.copyWith(callId: () => callId).sendSignalToRust(),
  );
}
