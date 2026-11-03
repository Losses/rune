import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<bool> clearLogs() async {
  final clearRequest = ClearLogRequest();

  // Listen for the response from Rust
  final rustSignal = await awaitResponse(
    'ClearLogRequest',
    ClearLogResponse.rustSignalStream,
    send: () => clearRequest.sendSignalToRust(),
  );
  final response = rustSignal.message;

  return response.success;
}
