import 'package:flutter_test/flutter_test.dart';

import '../bindings/bindings.dart';
import '../utils/api/operate_playback_with_mix_query.dart';

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
}
