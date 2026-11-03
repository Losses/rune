import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<String> getSSLCertificateFingerprint() async {
  final rustSignal = await awaitResponse(
    'GetSslCertificateFingerprintRequest',
    GetSslCertificateFingerprintResponse.rustSignalStream,
    send: () => GetSslCertificateFingerprintRequest().sendSignalToRust(),
  );

  return rustSignal.message.fingerprint;
}
