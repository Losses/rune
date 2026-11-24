import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<void> clearLogs() async {
  final clearRequest = ClearLogRequest();

  // Listen for the response from Rust
  await awaitResponse(
    'ClearLogRequest',
    ClearLogResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) =>
        clearRequest.copyWith(callId: () => callId).sendSignalToRust(),
  );
}
