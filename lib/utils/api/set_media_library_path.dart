import '../../constants/configurations.dart';
import '../../bindings/bindings.dart';

import '../settings_manager.dart';

(OperationDestination, OperationDestination) determineConnectionType(
    String path) {
  if (path.startsWith('@RR|')) {
    return (OperationDestination.remote, OperationDestination.remote);
  } else if (path.startsWith('@LR|')) {
    return (OperationDestination.local, OperationDestination.remote);
  }
  return (OperationDestination.local, OperationDestination.local);
}

LibraryInitializeMode? stringToLibraryInitializeMode(String? x) {
  if (x == null) return null;
  if (x == 'Redirected') return LibraryInitializeMode.redirected;

  return LibraryInitializeMode.portable;
}

Future<(bool, bool, String?)> setMediaLibraryPath(
  String path,
  LibraryInitializeMode? mode,
) async {
  final (playsOn, hostedOn) = determineConnectionType(path);

  final cleanPath = path.startsWith('@RR|') || path.startsWith('@LR|')
      ? path.substring(4)
      : path;

  final settingsManager = SettingsManager();

  SetMediaLibraryPathRequest(
    path: cleanPath,
    dbPath: await getSettingsPath(),
    configPath: await getSettingsPath(),
    alias: await settingsManager.getValue<String?>(kDeviceAliasKey) ?? "Rune",
    mode: mode,
    playsOn: playsOn,
    hostedOn: hostedOn,
  ).sendSignalToRust();

  while (true) {
    final rustSignal = await SetMediaLibraryPathResponse.rustSignalStream.first;
    final response = rustSignal.message;

    if (response.path == cleanPath) {
      return (response.success, response.notReady, response.error);
    }
  }
}
