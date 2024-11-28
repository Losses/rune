import '../../messages/all.dart';

import '../settings_manager.dart';

LibraryInitializeMode? stringToLibraryInitializeMode(String? x) {
  if (x == null) return null;
  if (x == 'Redirected') return LibraryInitializeMode.Redirected;

  return LibraryInitializeMode.Portable;
}

Future<(bool, bool, String?)> setMediaLibraryPath(
  String path,
  LibraryInitializeMode? mode,
) async {
  SetMediaLibraryPathRequest(
    path: path,
    dbPath: await getSettingsPath(),
    mode: mode,
  ).sendSignalToRust();

  while (true) {
    final rustSignal = await SetMediaLibraryPathResponse.rustSignalStream.first;
    final response = rustSignal.message;

    if (response.path == path) {
      return (response.success, response.notReady, response.error);
    }
  }
}
