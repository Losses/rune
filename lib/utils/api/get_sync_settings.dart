import 'request_failure.dart';
import '../../bindings/bindings.dart';

int _sequence = 0;

Future<GetSyncSettingsResponse> getSyncSettings() async {
  final id = '${DateTime.now().microsecondsSinceEpoch}-${_sequence++}';
  final response = awaitResponse(
    'GetSyncSettingsRequest',
    GetSyncSettingsResponse.rustSignalStream.where(
      (event) => event.message.requestId == id,
    ),
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) => GetSyncSettingsRequest(
      requestId: id,
      callId: callId,
    ).sendSignalToRust(),
    timeout: const Duration(seconds: 15),
  );
  return (await response).message;
}
