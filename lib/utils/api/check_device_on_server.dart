import 'request_failure.dart';
import '../../bindings/bindings.dart';
import '../../constants/configurations.dart';

import '../settings_manager.dart';

Future<CheckDeviceOnServerResponse> checkDeviceOnServer(
    List<String> hosts, String fingerprint) async {
  final settingsManager = SettingsManager();

  final registerRequest = CheckDeviceOnServerRequest(
    alias: await settingsManager.getValue(kDeviceAliasKey),
    hosts: hosts,
    fingerprint: fingerprint,
  );

  final rustSignal = await awaitResponse(
    'CheckDeviceOnServerRequest',
    CheckDeviceOnServerResponse.rustSignalStream,
    send: () => registerRequest.sendSignalToRust(),
  );
  final response = rustSignal.message;

  return response;
}
