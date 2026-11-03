import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<String> fetchServerCertificate(String host) async {
  final removeTrustRequest = FetchServerCertificateRequest(
    url: 'https://$host/ping',
  );

  final rustSignal = await awaitResponse(
    'FetchServerCertificateRequest',
    FetchServerCertificateResponse.rustSignalStream,
    send: () => removeTrustRequest.sendSignalToRust(),
  );
  final response = rustSignal.message;

  if (!response.success) {
    throw response.error;
  }

  return response.fingerprint;
}
