import '../../bindings/bindings.dart';

Future<bool> removeTrustedClient(String fingerprint) async {
  final removeTrustRequest =
      RemoveTrustedClientRequest(fingerprint: fingerprint);
  removeTrustRequest.sendSignalToRust();

  final rustSignal = await RemoveTrustedClientResponse.rustSignalStream.first;
  final response = rustSignal.message;

  if (!response.success) {
    throw response.error;
  }

  return response.success;
}
