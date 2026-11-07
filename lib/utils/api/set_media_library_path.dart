import 'request_failure.dart';
import 'dart:async';

import '../../constants/configurations.dart';
import '../../bindings/bindings.dart';

import '../settings_manager.dart';

LibraryInitializeMode? stringToLibraryInitializeMode(String? x) {
  if (x == null) return null;
  if (x == 'Redirected') return LibraryInitializeMode.redirected;

  return LibraryInitializeMode.portable;
}

Future<(bool, bool, String?, LibraryOpenErrorKind?)> setMediaLibraryPath(
  String path,
  LibraryInitializeMode? mode,
) async {
  final settingsManager = SettingsManager();

  final request = SetMediaLibraryPathRequest(
    path: path,
    dbPath: await getSettingsPath(),
    configPath: await getSettingsPath(),
    alias: await settingsManager.getValue<String?>(kDeviceAliasKey) ?? "Rune",
    mode: mode,
    playsOn: OperationDestination.local,
    hostedOn: OperationDestination.local,
  );

  try {
    while (true) {
      final rustSignal = await awaitResponse(
        'SetMediaLibraryPathRequest',
        SetMediaLibraryPathResponse.rustSignalStream.where(
          (event) => event.message.path == path,
        ),
        send: () => request.sendSignalToRust(),
      );
      final response = rustSignal.message;

      if (response.path == path) {
        return (
          response.success,
          response.notReady,
          response.error,
          response.errorKind,
        );
      }
    }
  } on TimeoutException {
    return (
      false,
      false,
      'No response from the Rust backend within 30 seconds. '
          'The Rust side may have failed to initialize; check logcat for details.',
      null,
    );
  }
}
