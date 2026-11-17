import 'dart:async';
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
  // Holds the runner's answer to `copiesDir` back until it completes.
  Completer<void>? runnerBusy;
  // What each import told the user: null for an imported scene, or the error.
  late List<String?> reported;

  setUp(() async {
    temp = await Directory.systemTemp.createTemp('rune-scene-files-');
    runnerCopies = null;
    runnerBusy = null;
    reported = [];
    messenger.setMockMethodCallHandler(_channel, (call) async {
      if (call.method == 'copiesDir') await runnerBusy?.future;
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

  /// An import that ends as the provider's does: it reports the scene or the
  /// [error] it handled, and answers whether the import is over.
  Future<bool> Function(String path) import({
    String? error,
    bool answered = true,
  }) => (path) async {
    reported.add(error);
    return answered;
  };

  // A completed import, and one that failed with an error it handled.
  const outcomes = [null, 'Not a scene'];

  test(
    'a scene file opened from its own folder stays, with the folder',
    () async {
      final scene = await write('scene-imports/Project/Night.runescene');
      final sibling = await write('scene-imports/Project/notes.txt');

      for (final error in outcomes) {
        await SceneFiles.importOpened(
          await open(scene.path),
          import(error: error),
        );

        expect(scene.existsSync(), isTrue);
        expect(sibling.existsSync(), isTrue);
      }
      expect(reported, outcomes);
    },
  );

  test(
    'the copy a runner made is deleted with the folder made for it',
    () async {
      final copies = Directory(p.join(temp.path, 'cache', 'scene-imports'));
      runnerCopies = copies.path;
      final importing = await write('cache/scene-imports/c21e/Dusk.runescene');

      for (final error in outcomes) {
        final copy = await write('cache/scene-imports/7f3a/Night.runescene');

        await SceneFiles.importOpened(
          await open(copy.path),
          import(error: error),
        );

        expect(copy.parent.existsSync(), isFalse);
        expect(importing.existsSync(), isTrue);
        expect(copies.existsSync(), isTrue);
      }
      expect(reported, outcomes);
    },
  );

  test('the copy stays while the import has not answered', () async {
    runnerCopies = p.join(temp.path, 'cache', 'scene-imports');
    final copy = await write('cache/scene-imports/7f3a/Night.runescene');

    await SceneFiles.importOpened(
      await open(copy.path),
      import(error: 'No answer', answered: false),
    );

    expect(copy.existsSync(), isTrue);
    expect(reported, ['No answer']);
  });

  test('the outcome is reported before the runner names its folder', () async {
    runnerCopies = p.join(temp.path, 'cache', 'scene-imports');
    final busy = runnerBusy = Completer<void>();
    final copy = await write('cache/scene-imports/7f3a/Night.runescene');
    final told = Completer<void>();

    final done = SceneFiles.importOpened(await open(copy.path), (path) async {
      told.complete();
      return true;
    });
    await told.future;
    expect(copy.existsSync(), isTrue);

    busy.complete();
    await done;
    expect(copy.parent.existsSync(), isFalse);
  });

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
        await SceneFiles.importOpened(await open(path), import());
      }

      expect([
        for (final file in kept) file.existsSync(),
      ], everyElement(isTrue));
    },
  );
}
