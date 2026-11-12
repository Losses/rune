import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<String> getSSLCertificateFingerprint() async {
  final rustSignal = await awaitResponse(
    'GetSslCertificateFingerprintRequest',
    GetSslCertificateFingerprintResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) =>
        GetSslCertificateFingerprintRequest(callId: callId).sendSignalToRust(),
  );

  return rustSignal.message.fingerprint;
}
