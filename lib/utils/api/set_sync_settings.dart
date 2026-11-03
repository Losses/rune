import 'request_failure.dart';
import '../../bindings/bindings.dart';

int _sequence = 0;

Future<void> setSyncSettings(
  bool listen,
  MissingHostFilePolicy missingHostFile,
) async {
  final id = '${DateTime.now().microsecondsSinceEpoch}-${_sequence++}';
  final response = awaitResponse(
    'SetSyncSettingsRequest',
    SetSyncSettingsResponse.rustSignalStream.where(
      (event) => event.message.requestId == id,
    ),
    send: () => SetSyncSettingsRequest(
      requestId: id,
      listen: listen,
      missingHostFile: missingHostFile,
    ).sendSignalToRust(),
    timeout: const Duration(seconds: 15),
  );
  final error = (await response).message.error;
  if (error != null) throw StateError(error);
}
