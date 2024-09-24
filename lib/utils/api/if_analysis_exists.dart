import '../../messages/analyse.pbserver.dart';

Future<bool> ifAnalyseExists(int fileId) async {
  final fetchRequest = IfAnalyseExistsRequest(fileId: fileId);
  fetchRequest.sendSignalToRust(); // GENERATED

  // Listen for the response from Rust
  final rustSignal = await IfAnalyseExistsResponse.rustSignalStream.first;
  final response = rustSignal.message;

  return response.exists;
}
