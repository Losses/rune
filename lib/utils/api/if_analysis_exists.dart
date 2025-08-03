import '../../bindings/bindings.dart';

Future<bool> ifAnalyzeExists(int fileId) async {
  final fetchRequest = IfAnalyzeExistsRequest(fileId: fileId);
  fetchRequest.sendSignalToRust(); // GENERATED

  // Listen for the response from Rust
  final rustSignal = await IfAnalyzeExistsResponse.rustSignalStream.first;
  final response = rustSignal.message;

  return response.exists;
}
