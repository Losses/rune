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
    send: () => GetSyncSettingsRequest(requestId: id).sendSignalToRust(),
    timeout: const Duration(seconds: 15),
  );
  final result = (await response).message;
  if (result.error != null) throw StateError(result.error!);
  return result;
}
