import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<bool> ifAnalyzeExists(int fileId) async {
  final fetchRequest = IfAnalyzeExistsRequest(fileId: fileId);

  // Listen for the response from Rust
  final rustSignal = await awaitResponse(
    'IfAnalyzeExistsRequest',
    IfAnalyzeExistsResponse.rustSignalStream,
    send: () => fetchRequest.sendSignalToRust(),
  );
  final response = rustSignal.message;

  return response.exists;
}
