import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:rune/utils/settings_manager.dart';
import 'package:rune/utils/storage_key_manager.dart';

void main() {
  group('castSetting', () {
    test('keeps a value of the expected type', () {
      expect(castSetting<int>('k', 3), 3);
      expect(castSetting<String>('k', 'dark'), 'dark');
      expect(castSetting<List<dynamic>>('k', ['a']), ['a']);
      expect(castSetting<bool>('k', null), isNull);
    });

    test('drops a value of another type instead of throwing', () {
      expect(castSetting<int>('k', '3'), isNull);
      expect(castSetting<String>('k', 3), isNull);
      expect(castSetting<bool>('k', 'true'), isNull);
      expect(castSetting<List<dynamic>>('k', {'path': '/music'}), isNull);
    });
  });

  group('stored settings', () {
    late SettingsManager settings;

    setUpAll(() async {
      TestWidgetsFlutterBinding.ensureInitialized();
      final directory = await Directory.systemTemp.createTemp('rune-settings');
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(
            const MethodChannel('plugins.flutter.io/path_provider'),
            (call) async => directory.path,
          );
      addTearDown(() async {
        // GetStorage retains an open file handle until process exit on Windows.
        // The Windows test runner removes the reported directory after exit.
        if (!Platform.isWindows) await directory.delete(recursive: true);
      });
      StorageKeyManager.initialize(null);
      settings = SettingsManager();
    });

    test('getValue ignores a stored value of the wrong type', () async {
      await settings.setValue('test_wrong_type', 'three');
      expect(await settings.getValue<int>('test_wrong_type'), isNull);
      expect(await settings.getValue<String>('test_wrong_type'), 'three');
    });
  });
}
