import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<SystemInfoResponse> systemInfo() async {
  final updateRequest = SystemInfoRequest();

  final rustSignal = await awaitResponse(
    'SystemInfoRequest',
    SystemInfoResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) =>
        updateRequest.copyWith(callId: () => callId).sendSignalToRust(),
  );
  final response = rustSignal.message;

  return response;
}
