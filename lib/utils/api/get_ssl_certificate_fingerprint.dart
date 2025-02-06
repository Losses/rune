import '../../messages/all.dart';

Future<String> getSSLCertificateFingerprint() async {
  GetSSLCertificateFingerprintRequest().sendSignalToRust();

  final rustSignal =
      await GetSSLCertificateFingerprintResponse.rustSignalStream.first;

  return rustSignal.message.fingerprint;
}
