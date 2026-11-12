import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<void> addItemToMix(int mixId, String operator, String parameter) async {
  final request = AddItemToMixRequest(
    mixId: mixId,
    operator: operator,
    parameter: parameter,
  );

  await awaitResponse(
    'AddItemToMixRequest',
    AddItemToMixResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) => request.copyWith(callId: () => callId).sendSignalToRust(),
  );
}
