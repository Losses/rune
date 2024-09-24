import '../../messages/analyse.pbserver.dart';

ifAnalyseExists(int fileId) async {
  final fetchRequest = GetAnalyseCountRequest();
  fetchRequest.sendSignalToRust(); // GENERATED

  // Listen for the response from Rust
  final rustSignal = await GetAnalyseCountResponse.rustSignalStream.first;
  final response = rustSignal.message;

  return response.count.toInt();
}
