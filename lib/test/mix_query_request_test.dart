import 'package:flutter_test/flutter_test.dart';

import '../bindings/bindings.dart';
import '../utils/api/query_mix_tracks.dart';
import '../utils/query_list.dart';

void main() {
  PlaylistItem entry(String track) => PlaylistItem(
    item: PlayingItemRefLibraryTrack(libraryId: 'library', track: track),
    artist: '',
    album: '',
    title: '',
    duration: 0,
  );

  test('a mix page request carries the queue the UI shows', () {
    final request = mixQueryRequest(const QueryList(), null, null, [
      entry('first'),
      entry('second'),
    ]);
    expect(
      request.queue.items.cast<PlayingItemRefLibraryTrack>().map(
        (item) => item.track,
      ),
      ['first', 'second'],
    );
  });

  test('a mix page request without a queue carries an empty one', () {
    expect(
      mixQueryRequest(const QueryList(), null, null, null).queue.items,
      isEmpty,
    );
  });

  test('a mix page request forwards randomSeed when provided', () {
    final seeded = mixQueryRequest(
      const QueryList(),
      0,
      100,
      null,
      randomSeed: 42,
    );
    expect(seeded.randomSeed?.toInt(), 42);

    final unseeded = mixQueryRequest(const QueryList(), 0, 100, null);
    expect(unseeded.randomSeed, isNull);
  });
}
