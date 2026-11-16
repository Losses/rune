import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<void> removeLog(int id) async {
  final removeRequest = RemoveLogRequest(id: id);

  // Listen for the response from Rust
  await awaitResponse(
    'RemoveLogRequest',
    RemoveLogResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) =>
        removeRequest.copyWith(callId: () => callId).sendSignalToRust(),
  );
}
