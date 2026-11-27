import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

import 'settings_folder.dart';

void main() {
  const name = 'rune-settings-folder-test';

  tearDown(() async {
    final folder = Directory('${Directory.systemTemp.path}/$name');
    if (folder.existsSync()) await folder.delete(recursive: true);
  });

  test('the folder is made where there is none', () async {
    final folder = await emptySettingsFolder(name);
    expect(folder.existsSync(), isTrue);
    expect(folder.listSync(), isEmpty);
  });

  test('what an earlier run left in the folder is gone', () async {
    final earlier = await emptySettingsFolder(name);
    await File('${earlier.path}/rune.gs').writeAsString('{"key":1}');
    await File('${earlier.path}/rune.bak').writeAsString('{"key":1}');

    final folder = await emptySettingsFolder(name);
    expect(folder.path, earlier.path);
    expect(folder.listSync(), isEmpty);
  });
}
