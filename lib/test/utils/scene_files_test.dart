import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:path/path.dart' as p;
import 'package:rune/utils/scene_files.dart';

const _channel = MethodChannel('ci.not.rune/scene_files');

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final messenger =
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;

  late Directory temp;
  // The folder the runner says it copies opened files into; null for a runner
  // that hands over the user's own files and does not answer `copiesDir`.
  String? runnerCopies;

  setUp(() async {
    temp = await Directory.systemTemp.createTemp('rune-scene-files-');
    runnerCopies = null;
    messenger.setMockMethodCallHandler(_channel, (call) async {
      return switch (call.method) {
        'takePending' => const <String>[],
        'copiesDir' when runnerCopies != null => runnerCopies,
        _ => throw MissingPluginException(),
      };
    });
    await SceneFiles.start(const []);
  });

  tearDown(() async {
    messenger.setMockMethodCallHandler(_channel, null);
    await temp.delete(recursive: true);
  });

  Future<File> write(String path) async {
    final file = File(p.join(temp.path, path));
    await file.create(recursive: true);
    return file.writeAsString('kept');
  }

  /// Has the runner hand [path] over, and returns it as Dart receives it.
  Future<String> open(String path) async {
    final opened = SceneFiles.opened.first;
    await messenger.handlePlatformMessage(
      _channel.name,
      _channel.codec.encodeMethodCall(MethodCall('open', [path])),
      (_) {},
    );
    return opened;
  }

  test(
    'a scene file opened from its own folder stays, with the folder',
    () async {
      final scene = await write('scene-imports/Project/Night.runescene');
      final sibling = await write('scene-imports/Project/notes.txt');

      await SceneFiles.dropCopy(await open(scene.path));

      expect(scene.existsSync(), isTrue);
      expect(sibling.existsSync(), isTrue);
    },
  );

  test(
    'the copy a runner made is deleted with the folder made for it',
    () async {
      final copies = Directory(p.join(temp.path, 'cache', 'scene-imports'));
      runnerCopies = copies.path;
      final copy = await write('cache/scene-imports/7f3a/Night.runescene');
      final importing = await write('cache/scene-imports/c21e/Dusk.runescene');

      await SceneFiles.dropCopy(await open(copy.path));

      expect(copy.parent.existsSync(), isFalse);
      expect(importing.existsSync(), isTrue);
      expect(copies.existsSync(), isTrue);
    },
  );

  test(
    'a file that is not a copy of the runner stays, wherever it is',
    () async {
      final copies = Directory(p.join(temp.path, 'cache', 'scene-imports'));
      runnerCopies = copies.path;
      final kept = [
        await write('scene-imports/Project/Night.runescene'),
        await write('scene-imports/Project/notes.txt'),
        await write('cache/scene-imports/Loose.runescene'),
        await write('cache/scene-imports/7f3a/Nested/Deep.runescene'),
        await write('cache/scene-imports-old/7f3a/Old.runescene'),
      ];

      for (final path in [
        for (final file in kept) file.path,
        p.join(
          copies.path,
          '7f3a',
          '..',
          '..',
          '..',
          'scene-imports',
          'Project',
          'Night.runescene',
        ),
        p.join(copies.path, '.', 'Loose.runescene'),
      ]) {
        await SceneFiles.dropCopy(await open(path));
      }

      expect([
        for (final file in kept) file.existsSync(),
      ], everyElement(isTrue));
    },
  );
}
