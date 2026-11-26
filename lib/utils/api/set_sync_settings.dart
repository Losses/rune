import 'request_failure.dart';
import '../../bindings/bindings.dart';

int _sequence = 0;

Future<void> setSyncSettings(GetSyncSettingsResponse settings) async {
  final id = '${DateTime.now().microsecondsSinceEpoch}-${_sequence++}';
  await awaitResponse(
    'SetSyncSettingsRequest',
    SetSyncSettingsResponse.rustSignalStream.where(
      (event) => event.message.requestId == id,
    ),
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) => SetSyncSettingsRequest(
      requestId: id,
      mediaServer: settings.mediaServer,
      broadcast: settings.broadcast,
      acceptRemoteControl: settings.acceptRemoteControl,
      acceptSync: settings.acceptSync,
      missingHostFile: settings.missingHostFile,
      callId: callId,
    ).sendSignalToRust(),
    timeout: const Duration(seconds: 15),
  );
}
