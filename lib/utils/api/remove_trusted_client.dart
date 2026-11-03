import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<bool> removeTrustedClient(String fingerprint) async {
  final removeTrustRequest = RemoveTrustedClientRequest(
    fingerprint: fingerprint,
  );

  final rustSignal = await awaitResponse(
    'RemoveTrustedClientRequest',
    RemoveTrustedClientResponse.rustSignalStream,
    send: () => removeTrustRequest.sendSignalToRust(),
  );
  final response = rustSignal.message;

  if (!response.success) {
    throw response.error;
  }

  return response.success;
}
