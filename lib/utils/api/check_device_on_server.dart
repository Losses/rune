import '../../bindings/bindings.dart';
import '../../constants/configurations.dart';

import '../settings_manager.dart';

Future<CheckDeviceOnServerResponse> checkDeviceOnServer(
    List<String> hosts) async {
  final settingsManager = SettingsManager();

  final registerRequest = CheckDeviceOnServerRequest(
    alias: await settingsManager.getValue(kDeviceAliasKey),
    hosts: hosts,
  );
  registerRequest.sendSignalToRust();

  final rustSignal = await CheckDeviceOnServerResponse.rustSignalStream.first;
  final response = rustSignal.message;

  return response;
}
