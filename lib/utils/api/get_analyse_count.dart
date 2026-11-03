import 'request_failure.dart';
import '../../bindings/bindings.dart';

ifAnalyzeExists(int fileId) async {
  final fetchRequest = GetAnalyzeCountRequest();

  // Listen for the response from Rust
  final rustSignal = await awaitResponse(
    'GetAnalyzeCountRequest',
    GetAnalyzeCountResponse.rustSignalStream,
    send: () => fetchRequest.sendSignalToRust(),
  );
  final response = rustSignal.message;

  return response.count.toInt();
}
