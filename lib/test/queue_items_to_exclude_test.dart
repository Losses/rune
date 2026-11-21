import 'package:flutter_test/flutter_test.dart';

import '../bindings/bindings.dart';
import '../utils/api/operate_playback_with_mix_query.dart';
import '../utils/query_list.dart';

void main() {
  PlaylistItem entry(String track) => PlaylistItem(
    item: PlayingItemRefLibraryTrack(libraryId: 'library', track: track),
    artist: '',
    album: '',
    title: '',
    duration: 0,
  );

  test('a mix leaves out the whole queue of the player the UI shows', () {
    final excluded = queueItemsToExclude(
      castTo: null,
      queue: [entry('first'), entry('second')],
    );
    expect(excluded.length, 2);
    expect(
      excluded.cast<PlayingItemRefLibraryTrack>().map((item) => item.track),
      ['first', 'second'],
    );
  });

  test('a mix cast to another renderer leaves nothing out', () {
    expect(
      queueItemsToExclude(
        castTo: const CastTarget(fingerprint: 'renderer', attach: false),
        queue: [entry('first')],
      ),
      isEmpty,
    );
  });

  test('without a queue nothing is left out', () {
    expect(queueItemsToExclude(castTo: null, queue: null), isEmpty);
  });

  group('which mixes leave the queue out', () {
    const album = QueryList([('lib::album', '1')]);
    const recommendation = QueryList([
      ('lib::album', '1'),
      ('pipe::recommend', '30'),
    ]);

    test('playing an album or a track the queue holds keeps its tracks', () {
      expect(
        mixExcludesQueue(
          operateMode: PlaylistOperateMode.replace,
          queries: album,
        ),
        isFalse,
      );
    });

    test('adding to the queue leaves out what it already holds', () {
      for (final mode in [
        PlaylistOperateMode.appendToEnd,
        PlaylistOperateMode.playNext,
      ]) {
        expect(mixExcludesQueue(operateMode: mode, queries: album), isTrue);
      }
    });

    test('a recommendation mix leaves out the queue it replaces', () {
      expect(
        mixExcludesQueue(
          operateMode: PlaylistOperateMode.replace,
          queries: recommendation,
        ),
        isTrue,
      );
    });

    test('a recommendation mix leaves out the queue at any position', () {
      expect(
        mixExcludesQueue(
          operateMode: PlaylistOperateMode.replace,
          queries: const QueryList([('pipe::recommend', '30')]),
        ),
        isTrue,
      );
    });
  });
}
