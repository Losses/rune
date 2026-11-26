import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:rinf/rinf.dart';

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

  test('library requests carry the queue of the player the UI shows', () {
    PlaylistUpdate.latestRustSignal = RustSignalPack(
      PlaylistUpdate(
        items: [entry('first'), entry('second')],
        sessionId: 'session',
        revision: Uint64.fromBigInt(BigInt.one),
      ),
      Uint8List(0),
    );
    addTearDown(() => PlaylistUpdate.latestRustSignal = null);
    expect(
      shownPlayerQueue().items.cast<PlayingItemRefLibraryTrack>().map(
        (item) => item.track,
      ),
      ['first', 'second'],
    );
  });

  test(
    'before the player names a queue, library requests carry an empty one',
    () {
      PlaylistUpdate.latestRustSignal = null;
      expect(shownPlayerQueue().items, isEmpty);
    },
  );
}
