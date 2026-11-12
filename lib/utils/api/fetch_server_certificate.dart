import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<String> fetchServerCertificate(String host) async {
  final removeTrustRequest = FetchServerCertificateRequest(
    url: 'https://$host/ping',
  );

  final rustSignal = await awaitResponse(
    'FetchServerCertificateRequest',
    FetchServerCertificateResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) =>
        removeTrustRequest.copyWith(callId: () => callId).sendSignalToRust(),
  );
  final response = rustSignal.message;

  if (!response.success) {
    throw response.error;
  }

  return response.fingerprint;
}
