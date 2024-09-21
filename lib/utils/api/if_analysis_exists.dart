import 'package:player/messages/recommend.pbserver.dart';

Future<bool> ifAnalysisExists(int fileId) async {
  final fetchRequest = IfAnalysisExistsRequest(fileId: fileId);
  fetchRequest.sendSignalToRust(); // GENERATED

  // Listen for the response from Rust
  final rustSignal = await IfAnalysisExistsResponse.rustSignalStream.first;
  final response = rustSignal.message;

  return response.exists;
}
