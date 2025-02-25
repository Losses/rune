import '../../messages/all.dart';
import '../../constants/configurations.dart';

import '../settings_manager.dart';

registerDeviceOnServer() async {
  final settingsManager = SettingsManager();

  final registerRequest = RegisterDeviceOnServerRequest(
    alias: await settingsManager.getValue(kDeviceAliasKey),
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
