import 'package:flutter_test/flutter_test.dart';

import '../utils/api/operate_playback_with_mix_query.dart';
import '../utils/playing_item.dart';

void main() {
  const files = [
    PlayingItem.independentFile('/music/first.ogg'),
    PlayingItem.independentFile('/music/second.ogg'),
  ];
  const fallbackTracks = [PlayingItem.inLibrary(3), PlayingItem.inLibrary(5)];

  test('opened files with no track to start with ask for no catalog row', () {
    final items = playbackItemsToResolve(
      initialPlaybackId: null,
      fallbackPlayingItems: files,
    );

    expect(items, files);
    expect(items.where((item) => item.hasLocalFileId), isEmpty);
  });

  test('a collection with no track to start with asks only for its fallback '
      'tracks', () {
    final items = playbackItemsToResolve(
      initialPlaybackId: null,
      fallbackPlayingItems: fallbackTracks,
    );

    expect(items, fallbackTracks);
  });

  test('a track to start with comes first, before the fallback tracks', () {
    final items = playbackItemsToResolve(
      initialPlaybackId: 7,
      fallbackPlayingItems: fallbackTracks,
    );

    expect(items, [const PlayingItem.inLibrary(7), ...fallbackTracks]);
  });

  test('a track that was asked for and is not in the catalog is an error', () {
    expect(() => localTrackRef(7, const []), throwsStateError);
  });
}
