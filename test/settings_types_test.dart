import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:rune/constants/configurations.dart';
import 'package:rune/utils/file_storage/file_storage_service.dart';
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

  group('normalizeOpenedFiles', () {
    final service = FileStorageService();

    test('keeps current entries as they are', () {
      final entry = {
        'path': '/music',
        'alias': 'music',
        'createdAt': '2026-10-21T00:00:00.000',
      };
      expect(service.normalizeOpenedFiles([entry]), [entry]);
    });

    // A version of Rune before library aliases writes bare paths but leaves
    // the data version at 2
    test('migrates bare paths whatever the data version says', () {
      final entries = service.normalizeOpenedFiles([
        {'path': '/music', 'alias': 'music', 'createdAt': ''},
        '/old/library',
      ]);
      expect(entries.map((e) => e['path']), ['/music', '/old/library']);
      expect(entries.last['alias'], 'library');
    });

    test('drops entries without a path', () {
      final entries = service.normalizeOpenedFiles([
        null,
        3,
        {'alias': 'no path'},
        {'path': 3},
        ['/music'],
        '',
        {'path': ''},
        <dynamic, dynamic>{'path': '/other', 1: 'not a string key'},
        {'path': '/music'},
      ]);
      expect(entries, [
        {'path': '/music'},
      ]);
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
      addTearDown(() => directory.delete(recursive: true));
      StorageKeyManager.initialize(null);
      settings = SettingsManager();
    });

    test('getValue ignores a stored value of the wrong type', () async {
      await settings.setValue('test_wrong_type', 'three');
      expect(await settings.getValue<int>('test_wrong_type'), isNull);
      expect(await settings.getValue<String>('test_wrong_type'), 'three');
    });

    // A data version of the wrong type reads as version 1 and starts the
    // migration from bare paths
    test('migrating keeps entries that are already current', () async {
      await settings.setValue(kOpenedFilesKey, [
        {'path': '/music', 'alias': 'music', 'createdAt': ''},
        '/old/library',
      ]);
      await settings.setValue(kDataVersionKey, '2');
      expect(await FileStorageService().getAllOpenedFiles(), [
        '/music',
        '/old/library',
      ]);
    });
  });
}
