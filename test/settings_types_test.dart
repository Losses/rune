import 'package:flutter_test/flutter_test.dart';

import 'package:rune/utils/file_storage/file_storage_service.dart';
import 'package:rune/utils/settings_manager.dart';

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
      expect(entries.last['alias'], isA<String>());
    });

    test('drops entries without a path', () {
      final entries = service.normalizeOpenedFiles([
        null,
        3,
        {'alias': 'no path'},
        {'path': 3},
        ['/music'],
        {'path': '/music'},
      ]);
      expect(entries, [
        {'path': '/music'},
      ]);
    });
  });
}
