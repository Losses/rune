import '../../messages/all.dart';

Future<bool> removeTrust(String fingerprint) async {
  final removeTrustRequest = RemoveTrustRequest(fingerprint: fingerprint);
  removeTrustRequest.sendSignalToRust(); // GENERATED

  final rustSignal = await RemoveTrustResponse.rustSignalStream.first;
  final response = rustSignal.message;

  if (!response.success) {
    throw response.error;
  }

  return response.success;
}
