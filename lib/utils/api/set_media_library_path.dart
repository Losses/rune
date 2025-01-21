import '../../messages/all.dart';

import '../settings_manager.dart';

(OperationDestination, OperationDestination) determineConnectionType(
    String path) {
  if (path.startsWith('@RR|')) {
    return (OperationDestination.Remote, OperationDestination.Remote);
  } else if (path.startsWith('@LR|')) {
    return (OperationDestination.Local, OperationDestination.Remote);
  }
  return (OperationDestination.Local, OperationDestination.Local);
}

LibraryInitializeMode? stringToLibraryInitializeMode(String? x) {
  if (x == null) return null;
  if (x == 'Redirected') return LibraryInitializeMode.Redirected;

  return LibraryInitializeMode.Portable;
}

Future<(bool, bool, String?)> setMediaLibraryPath(
  String path,
  LibraryInitializeMode? mode,
) async {
  final (playsOn, hostedOn) = determineConnectionType(path);

  final cleanPath = path.startsWith('@RR|') || path.startsWith('@LR|')
      ? path.substring(4)
      : path;

  SetMediaLibraryPathRequest(
    path: cleanPath,
    dbPath: await getSettingsPath(),
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
