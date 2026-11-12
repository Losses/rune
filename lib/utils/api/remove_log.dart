import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<bool> removeLog(int id) async {
  final removeRequest = RemoveLogRequest(id: id);

  // Listen for the response from Rust
  final rustSignal = await awaitResponse(
    'RemoveLogRequest',
    RemoveLogResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) =>
        removeRequest.copyWith(callId: () => callId).sendSignalToRust(),
  );
  final response = rustSignal.message;

  return response.success;
}
