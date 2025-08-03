import '../../bindings/bindings.dart';
import '../../constants/configurations.dart';

import '../settings_manager.dart';

registerDeviceOnServer(List<String> hosts) async {
  final settingsManager = SettingsManager();

  final registerRequest = RegisterDeviceOnServerRequest(
    alias: await settingsManager.getValue(kDeviceAliasKey),
    hosts: hosts,
  );
  registerRequest.sendSignalToRust();

  final rustSignal =
      await RegisterDeviceOnServerResponse.rustSignalStream.first;
  final response = rustSignal.message;

  if (!response.success) {
    throw response.error;
  }

  return response.success;
}
