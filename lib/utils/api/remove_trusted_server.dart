import '../../bindings/bindings.dart';

Future<bool> removeTrustedServer(String fingerprint) async {
  final removeTrustRequest =
      RemoveTrustedServerRequest(fingerprint: fingerprint);
  removeTrustRequest.sendSignalToRust();

  final rustSignal = await RemoveTrustedServerResponse.rustSignalStream.first;
  final response = rustSignal.message;

  if (!response.success) {
    throw response.error;
  }

  return response.success;
}
