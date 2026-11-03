import 'request_failure.dart';
import '../../bindings/bindings.dart';
import '../../constants/configurations.dart';

import '../settings_manager.dart';

registerDeviceOnServer(List<String> hosts) async {
  final settingsManager = SettingsManager();

  final registerRequest = RegisterDeviceOnServerRequest(
    alias: await settingsManager.getValue(kDeviceAliasKey),
    hosts: hosts,
  );

  final rustSignal = await awaitResponse(
    'RegisterDeviceOnServerRequest',
    RegisterDeviceOnServerResponse.rustSignalStream,
    send: () => registerRequest.sendSignalToRust(),
  );
  final response = rustSignal.message;

  if (!response.success) {
    throw response.error;
  }

  return response.success;
}
