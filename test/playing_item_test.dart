import 'package:flutter_test/flutter_test.dart';

import 'package:rune/utils/playing_item.dart';

void main() {
  // Responses are matched to requests by comparing the item that comes back
  test('items survive the round trip through a request', () {
    for (final item in const [
      PlayingItem.inLibrary(42),
      PlayingItem.independentFile('/Music/outside/song.flac'),
      PlayingItem.unknown(),
    ]) {
      expect(PlayingItem.fromRequest(item.toRequest()), item);
    }
  });

  test('an empty path is unknown, as in the hub', () {
    expect(
      PlayingItem.fromRequest(
        const PlayingItem.independentFile('').toRequest(),
      ),
      const PlayingItem.unknown(),
    );
  });

  test('items are read back from their status strings', () {
    for (final item in const [
      PlayingItem.inLibrary(42),
      PlayingItem.independentFile('/Music/outside/song (live).flac'),
      PlayingItem.unknown(),
    ]) {
      expect(PlayingItem.fromString(item.toString()), item);
    }
    // The Dart side has no online items
    expect(
      PlayingItem.fromString(
        'PlayingItem::Online(https://example.com/a, None)',
      ),
      const PlayingItem.unknown(),
    );
  });
}
