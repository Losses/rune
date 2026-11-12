import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<bool> removeMix(int mixId) async {
  final updateRequest = RemoveMixRequest(mixId: mixId);

  final rustSignal = await awaitResponse(
    'RemoveMixRequest',
    RemoveMixResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) =>
        updateRequest.copyWith(callId: () => callId).sendSignalToRust(),
  );
  final response = rustSignal.message;

  return response.success;
}
