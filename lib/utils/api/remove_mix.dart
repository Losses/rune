import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<bool> removeMix(int mixId) async {
  final updateRequest = RemoveMixRequest(mixId: mixId);

  final rustSignal = await awaitResponse(
    'RemoveMixRequest',
    RemoveMixResponse.rustSignalStream,
    send: () => updateRequest.sendSignalToRust(),
  );
  final response = rustSignal.message;

  return response.success;
}
