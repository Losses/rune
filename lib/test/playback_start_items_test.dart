import 'package:flutter_test/flutter_test.dart';

import '../bindings/bindings.dart';
import '../utils/api/operate_playback_with_mix_query.dart';
import '../utils/playing_item.dart';

// No library is open in these tests, so a catalog lookup would fail them:
// they pass only when none is made.
void main() {
  const files = [
    PlayingItem.independentFile('/music/first.ogg'),
    PlayingItem.independentFile('/music/second.ogg'),
  ];

  test(
    'opened files with no track to start with need no catalog row',
    () async {
      final start = await playbackStartItems(
        initialPlaybackId: null,
        fallbackPlayingItems: files,
      );

      expect(start.initial, isNull);
      expect(start.fallback, const [
        PlayingItemRefIndependentFile(path: '/music/first.ogg'),
        PlayingItemRefIndependentFile(path: '/music/second.ogg'),
      ]);
    },
  );

  test('a collection with no track to start with and no fallback tracks '
      'needs no catalog row', () async {
    final start = await playbackStartItems(
      initialPlaybackId: null,
      fallbackPlayingItems: const [],
    );

    expect(start.initial, isNull);
    expect(start.fallback, isEmpty);
  });

  test('a track that was asked for and is not in the catalog is an error', () {
    expect(() => localTrackRef(7, const []), throwsStateError);
  });
}
