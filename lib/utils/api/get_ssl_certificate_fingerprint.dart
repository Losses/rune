import '../../messages/all.dart';

Future<String> getSSLCertificateFingerprint() async {
  GetSslCertificateFingerprintRequest().sendSignalToRust();

  final rustSignal =
      await GetSslCertificateFingerprintResponse.rustSignalStream.first;

  return rustSignal.message.fingerprint;
}
