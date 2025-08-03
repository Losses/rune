import '../../bindings/bindings.dart';

ifAnalyzeExists(int fileId) async {
  final fetchRequest = GetAnalyzeCountRequest();
  fetchRequest.sendSignalToRust(); // GENERATED

  // Listen for the response from Rust
  final rustSignal = await GetAnalyzeCountResponse.rustSignalStream.first;
  final response = rustSignal.message;

  return response.count.toInt();
}
