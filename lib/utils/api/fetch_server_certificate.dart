import '../../bindings/bindings.dart';

Future<String> fetchServerCertificate(String host) async {
  final removeTrustRequest =
      FetchServerCertificateRequest(url: 'https://$host/ping');
  removeTrustRequest.sendSignalToRust();

  final rustSignal =
      await FetchServerCertificateResponse.rustSignalStream.first;
  final response = rustSignal.message;

  if (!response.success) {
    throw response.error;
  }

  return response.fingerprint;
}
