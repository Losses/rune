import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<void> removeMix(int mixId) async {
  final updateRequest = RemoveMixRequest(mixId: mixId);

  await awaitResponse(
    'RemoveMixRequest',
    RemoveMixResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) =>
        updateRequest.copyWith(callId: () => callId).sendSignalToRust(),
  );
}
