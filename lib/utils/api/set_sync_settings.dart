import 'request_failure.dart';
import '../../bindings/bindings.dart';

int _sequence = 0;

/// The sync settings one save changes. A null field is left as the hub holds
/// it, so that a save never writes back a value it only read.
class SyncSettingsChange {
  const SyncSettingsChange({
    this.mediaServer,
    this.broadcast,
    this.acceptRemoteControl,
    this.acceptSync,
    this.autoDeviceSync,
    this.autoShareSync,
    this.missingHostFile,
  });

  final bool? mediaServer;
  final bool? broadcast;
  final bool? acceptRemoteControl;
  final bool? acceptSync;
  final bool? autoDeviceSync;
  final bool? autoShareSync;
  final MissingHostFilePolicy? missingHostFile;

  /// Returns [settings] with the fields this change sets replaced.
  GetSyncSettingsResponse applyTo(GetSyncSettingsResponse settings) =>
      settings.copyWith(
        mediaServer: mediaServer,
        broadcast: broadcast,
        acceptRemoteControl: acceptRemoteControl,
        acceptSync: acceptSync,
        autoDeviceSync: autoDeviceSync,
        autoShareSync: autoShareSync,
        missingHostFile: missingHostFile,
      );
}

Future<void> setSyncSettings(SyncSettingsChange change) async {
  final id = '${DateTime.now().microsecondsSinceEpoch}-${_sequence++}';
  await awaitResponse(
    'SetSyncSettingsRequest',
    SetSyncSettingsResponse.rustSignalStream.where(
      (event) => event.message.requestId == id,
    ),
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) => SetSyncSettingsRequest(
      requestId: id,
      mediaServer: change.mediaServer,
      broadcast: change.broadcast,
      acceptRemoteControl: change.acceptRemoteControl,
      acceptSync: change.acceptSync,
      autoDeviceSync: change.autoDeviceSync,
      autoShareSync: change.autoShareSync,
      missingHostFile: change.missingHostFile,
      callId: callId,
    ).sendSignalToRust(),
    timeout: const Duration(seconds: 15),
  );
}
