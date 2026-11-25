import 'dart:convert';
import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:rune/utils/file_storage/mac_secure_manager.dart';

void main() {
  late Directory directory;
  final accessed = <String>[];

  setUpAll(() async {
    TestWidgetsFlutterBinding.ensureInitialized();
    directory = await Directory.systemTemp.createTemp('rune-bookmarks');
    addTearDown(() => directory.delete(recursive: true));

    // One library folder still exists, the other one was deleted
    File(
      '${directory.path}/${MacSecureManager.storageKey}.gs',
    ).writeAsStringSync(
      jsonEncode({'/music/gone': 'gone', '/music/here': 'here'}),
    );

    final messenger =
        TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
    messenger.setMockMethodCallHandler(
      const MethodChannel('plugins.flutter.io/path_provider'),
      (call) async => directory.path,
    );
    // Behaves like the macOS plugin, which fails to resolve a bookmark whose
    // folder no longer exists
    messenger.setMockMethodCallHandler(
      const MethodChannel('codeux.design/macos_secure_bookmarks'),
      (call) async {
        final args = call.arguments as Map;
        switch (call.method) {
          case 'URLByResolvingBookmarkData':
            if (args['bookmark'] == 'gone') {
              throw PlatformException(
                code: 'UnexpectedError',
                message: 'Error while resolving bookmark',
              );
            }
            return '/music/${args['bookmark']}';
          case 'startAccessingSecurityScopedResource':
            accessed.add(args['file'] as String);
            return true;
        }
        return null;
      },
    );
  });

  test('a deleted library folder does not stop startup', () async {
    await expectLater(MacSecureManager().completed, completes);
    expect(accessed, ['/music/here']);

    // The bookmark is kept in case the folder comes back
    final stored = File('${directory.path}/${MacSecureManager.storageKey}.gs');
    expect(jsonDecode(stored.readAsStringSync()), contains('/music/gone'));
  }, skip: !MacSecureManager.isApplePlatform());
}
