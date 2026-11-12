import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<bool> clearLogs() async {
  final clearRequest = ClearLogRequest();

  // Listen for the response from Rust
  final rustSignal = await awaitResponse(
    'ClearLogRequest',
    ClearLogResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) =>
        clearRequest.copyWith(callId: () => callId).sendSignalToRust(),
  );
  final response = rustSignal.message;

  return response.success;
}
