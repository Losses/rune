import 'package:flutter_test/flutter_test.dart';

import '../../bindings/bindings.dart';
import '../../utils/context_menu/utils/collection_file_ids.dart';
import '../../widgets/track_list/utils/internal_media_file.dart';

InternalMediaFile _file(int id) => InternalMediaFile(
  id: id,
  libraryId: 'library',
  track: 't$id',
  path: '',
  artist: '',
  album: '',
  title: '',
  duration: 0,
  coverArtPath: '',
  trackNumber: null,
  audioAvailability: AudioAvailability.local,
  audioOrigin: AudioOrigin.host,
);

void main() {
  for (final type in [CollectionType.album, CollectionType.genre]) {
    test('the songs of a $type are asked in track order', () async {
      final asked = <List<(String, String)>>[];
      await collectionFileIds(
        type,
        1,
        loadQueries: (type, id) async => [('lib::${type.name}', '$id')],
        loadPage: (queries, cursor, pageSize, seed) async {
          asked.add(queries.toList());
          return const [];
        },
      );

      expect(asked.single, [
        ('lib::${type.name}', '1'),
        ('sort::track_number', 'true'),
      ]);
    });
  }

  test('a mix keeps the order of its own queries', () async {
    final asked = <List<(String, String)>>[];
    await collectionFileIds(
      CollectionType.mix,
      1,
      loadQueries: (type, id) async => [('lib::random', '30')],
      loadPage: (queries, cursor, pageSize, seed) async {
        asked.add(queries.toList());
        return const [];
      },
    );

    expect(asked.single, [('lib::random', '30')]);
  });

  test('pages are read until an empty one, with one seed', () async {
    final seeds = <int>{};
    final pages = [
      [_file(12), _file(11)],
      [_file(13)],
      <InternalMediaFile>[],
    ];
    final ids = await collectionFileIds(
      CollectionType.album,
      1,
      loadQueries: (type, id) async => const [('lib::album', '1')],
      loadPage: (queries, cursor, pageSize, seed) async {
        seeds.add(seed);
        return pages.removeAt(0);
      },
    );

    expect(ids, [12, 11, 13]);
    expect(seeds, hasLength(1));
  });
}
