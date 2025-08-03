import '../../bindings/bindings.dart';

Future<bool> serverAvailabilityTest(String url) async {
  final removeTrustRequest = ServerAvailabilityTestRequest(url: url);
  removeTrustRequest.sendSignalToRust();

  final rustSignal =
      await ServerAvailabilityTestResponse.rustSignalStream.first;
  final response = rustSignal.message;

  if (!response.success) {
    throw response.error;
  }

  return response.success;
}
