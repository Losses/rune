import '../../bindings/bindings.dart';

Future<bool> removeLog(int id) async {
  final removeRequest = RemoveLogRequest(
    id: id,
  );
  removeRequest.sendSignalToRust(); // GENERATED

  // Listen for the response from Rust
  final rustSignal = await RemoveLogResponse.rustSignalStream.first;
  final response = rustSignal.message;

  return response.success;
}
