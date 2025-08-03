import '../../bindings/bindings.dart';

Future<bool> clearLogs() async {
  final clearRequest = ClearLogRequest();
  clearRequest.sendSignalToRust();

  // Listen for the response from Rust
  final rustSignal = await ClearLogResponse.rustSignalStream.first;
  final response = rustSignal.message;

  return response.success;
}
