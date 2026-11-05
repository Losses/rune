import 'package:flutter_test/flutter_test.dart';

import 'package:rune/utils/playing_item.dart';
import 'package:rune/bindings/bindings.dart';

void main() {
  // Responses are matched to requests by comparing the item that comes back
  test('items survive the round trip through a request', () {
    for (final item in const [
      PlayingItem.inLibrary(42),
      PlayingItem.libraryItem(libraryId: 'library-uuid', fileUuid: 'file-uuid'),
      PlayingItem.independentFile('/Music/outside/song.flac'),
      PlayingItem.unknown(),
    ]) {
      expect(PlayingItem.fromRequest(item.toRequest()), item);
    }
  });

  test('maps a library item request without inventing a local id', () {
    const request = PlayingItemRequest(
      libraryItem: LibraryItemRequest(
        libraryId: 'library-uuid',
        fileUuid: 'file-uuid',
      ),
    );

    final item = PlayingItem.fromRequest(request);
    expect(item.libraryId, 'library-uuid');
    expect(item.fileUuid, 'file-uuid');
    expect(item.inLibrary, isNull);
    expect(item.hasLocalFileId, isFalse);
    expect(item.toRequest(), request);
  });

  test('maps a local library item request with its integer id', () {
    const request = PlayingItemRequest(
      inLibrary: InLibraryPlayingItem(fileId: 42),
    );

    final item = PlayingItem.fromRequest(request);
    expect(item.inLibrary, 42);
    expect(item.hasLocalFileId, isTrue);
    expect(item.toRequest(), request);
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
