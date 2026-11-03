import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<bool> serverAvailabilityTest(String url) async {
  final removeTrustRequest = ServerAvailabilityTestRequest(url: url);

  final rustSignal = await awaitResponse(
    'ServerAvailabilityTestRequest',
    ServerAvailabilityTestResponse.rustSignalStream,
    send: () => removeTrustRequest.sendSignalToRust(),
  );
  final response = rustSignal.message;

  if (!response.success) {
    throw response.error;
  }

  return response.success;
}
