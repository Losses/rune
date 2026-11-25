import 'dart:async';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:path_provider_platform_interface/path_provider_platform_interface.dart';
import 'package:rinf/rinf.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/constants/configurations.dart';
import 'package:rune/providers/broadcast.dart';
import 'package:rune/utils/api/request_failure.dart';
import 'package:rune/utils/settings_manager.dart';
import 'package:rune/utils/storage_key_manager.dart';

class _Paths extends PathProviderPlatform {
  _Paths(this.path);
  final String path;

  @override
  Future<String> getApplicationDocumentsPath() async => path;

  @override
  Future<String> getApplicationSupportPath() async => path;
}

Future<void> _waitForFingerprint(BroadcastProvider provider) async {
  if (provider.fingerprint != null) return;
  final ready = Completer<void>();
  void listener() {
    if (provider.fingerprint != null && !ready.isCompleted) ready.complete();
  }

  provider.addListener(listener);
  try {
    await ready.future.timeout(const Duration(seconds: 20));
  } finally {
    provider.removeListener(listener);
  }
}

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  testWidgets('an early device name is saved and blanks leave it intact', (
    tester,
  ) async {
    await tester.runAsync(() async {
      final root = await Directory.systemTemp.createTemp('rune-device-name');
      final previousPaths = PathProviderPlatform.instance;
      PathProviderPlatform.instance = _Paths(root.path);
      addTearDown(() async {
        PathProviderPlatform.instance = previousPaths;
        if (!Platform.isWindows) await root.delete(recursive: true);
      });
      StorageKeyManager.initialize('device-name-test');
      await initializeRust(assignRustSignal);
      addTearDown(finalizeRust);
      await awaitResponse(
        'InitializeNodeRequest',
        InitializeNodeResponse.rustSignalStream,
        responseCallId: (response) => response.message.callId?.toInt(),
        send: (callId) => InitializeNodeRequest(
          configPath: root.path,
          callId: callId,
        ).sendSignalToRust(),
      );

      final settings = SettingsManager();
      expect(await settings.getValue<String>(kDeviceAliasKey), isNull);
      final provider = BroadcastProvider();
      addTearDown(provider.dispose);
      await provider.updateDeviceAlias('Studio');
      await _waitForFingerprint(provider);
      expect(await settings.getValue<String>(kDeviceAliasKey), 'Studio');

      await provider.updateDeviceAlias('');
      await provider.updateDeviceAlias('  ');
      expect(provider.deviceAlias, 'Studio');
      expect(await settings.getValue<String>(kDeviceAliasKey), 'Studio');

      await settings.setValue(kDeviceAliasKey, '');
      final legacy = BroadcastProvider();
      addTearDown(legacy.dispose);
      await _waitForFingerprint(legacy);
      expect(legacy.deviceAlias, '');
      await legacy.updateDeviceAlias('Recovered studio');

      final reloaded = BroadcastProvider();
      addTearDown(reloaded.dispose);
      await _waitForFingerprint(reloaded);
      expect(reloaded.deviceAlias, 'Recovered studio');
      expect(
        await settings.getValue<String>(kDeviceAliasKey),
        'Recovered studio',
      );
    });
  });
}
