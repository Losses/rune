import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<SystemInfoResponse> systemInfo() async {
  final updateRequest = SystemInfoRequest();

  final rustSignal = await awaitResponse(
    'SystemInfoRequest',
    SystemInfoResponse.rustSignalStream,
    send: () => updateRequest.sendSignalToRust(),
  );
  final response = rustSignal.message;

  return response;
}
