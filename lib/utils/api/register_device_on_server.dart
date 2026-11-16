import 'request_failure.dart';
import '../../bindings/bindings.dart';
import '../../constants/configurations.dart';

import '../settings_manager.dart';

Future<void> registerDeviceOnServer(
  List<String> hosts,
  String fingerprint,
) async {
  final settingsManager = SettingsManager();

  final registerRequest = RegisterDeviceOnServerRequest(
    alias: await settingsManager.getValue(kDeviceAliasKey),
    hosts: hosts,
    fingerprint: fingerprint,
  );

  await awaitResponse(
    'RegisterDeviceOnServerRequest',
    RegisterDeviceOnServerResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) =>
        registerRequest.copyWith(callId: () => callId).sendSignalToRust(),
  );
}
