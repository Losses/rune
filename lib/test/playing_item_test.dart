import 'package:flutter_test/flutter_test.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/utils/playing_item.dart';
import 'package:rune/widgets/track_list/utils/internal_media_file.dart';

void main() {
  test(
    'local row id is converted to stable library identity before sending',
    () {
      final files = [
        InternalMediaFile(
          id: 42,
          libraryId: 'library-uuid',
          track: '5f891d9e-0a39-4d77-8ea0-3bcfedc4810e',
          path: '',
          artist: '',
          album: '',
          title: '',
          duration: 0,
          coverArtPath: '',
          trackNumber: null,
        ),
      ];

      expect(
        localTrackRef(42, files),
        const PlayingItemRefLibraryTrack(
          libraryId: 'library-uuid',
          track: '5f891d9e-0a39-4d77-8ea0-3bcfedc4810e',
        ),
      );
    },
  );

  test('library and Subsonic tracks share the same wire variant', () {
    const runeRef = PlayingItemRefLibraryTrack(
      libraryId: 'rune-library',
      track: '5f891d9e-0a39-4d77-8ea0-3bcfedc4810e',
    );
    const subsonicRef = PlayingItemRefLibraryTrack(
      libraryId: 'subsonic-library',
      track: 'opaque-track-id',
    );

    expect(
      PlayingItem.fromRequest(runeRef).fileUuid,
      '5f891d9e-0a39-4d77-8ea0-3bcfedc4810e',
    );
    expect(
      PlayingItem.fromRequest(subsonicRef).subsonicTrackId,
      'opaque-track-id',
    );
    expect(
      PlayingItem.libraryItem(libraryId: 'a', fileUuid: 'uuid').toRequest(),
      const PlayingItemRefLibraryTrack(libraryId: 'a', track: 'uuid'),
    );
  });

  test('independent files roundtrip through the wire enum', () {
    const item = PlayingItem.independentFile('/Music/outside/song.flac');
    expect(PlayingItem.fromRequest(item.toRequest()), item);
  });
}
