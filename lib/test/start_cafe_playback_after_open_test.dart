import 'dart:async';

import 'package:flutter_test/flutter_test.dart';

import '../utils/start_cafe_playback_after_open.dart';

void main() {
  test(
    'waits for a successful library open before starting playback once',
    () async {
      final opened = Completer<bool>();
      var playCount = 0;
      final startup = startCafePlaybackAfterOpen(
        libraryOpened: opened.future,
        play: () async => playCount++,
      );

      await Future<void>.delayed(Duration.zero);
      expect(playCount, 0);
      opened.complete(true);
      await startup;

      expect(playCount, 1);
    },
  );

  test('does not start playback when opening the library fails', () async {
    var playCount = 0;
    await startCafePlaybackAfterOpen(
      libraryOpened: Future.value(false),
      play: () async => playCount++,
    );
    expect(playCount, 0);
  });
}
