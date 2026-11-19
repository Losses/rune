import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';

import '../../bindings/bindings.dart';
import '../../utils/api/fetch_playlist_occurrences.dart';
import '../../utils/build_collection_query.dart';
import '../../utils/context_menu/utils/build_m3u8.dart';
import '../../widgets/track_list/utils/internal_media_file.dart';

InternalMediaFile _track(int index, {String? path}) => InternalMediaFile(
  id: index,
  libraryId: 'library',
  track: 'track-${index.toString().padLeft(4, '0')}',
  path: path ?? '/music/export-${index.toString().padLeft(4, '0')}.ogg',
  artist: 'Artist',
  album: 'Album',
  title: 'Export Track ${index.toString().padLeft(4, '0')}',
  duration: 1,
  coverArtPath: '',
  trackNumber: index,
);

List<String> _m3u8Lines(String content) =>
    const LineSplitter().convert(content).where((line) => line.isNotEmpty).toList();

Future<PlaylistOccurrencePage> _servePlaylistPage(
  List<InternalMediaFile> allTracks,
  PlaylistOccurrenceCursor? after,
  int pageSize,
) async {
  expect(pageSize, inInclusiveRange(1, 128));
  final start = after == null ? 0 : after.position + 1;
  final end = (start + pageSize).clamp(0, allTracks.length);
  final items = [
    for (var i = start; i < end; i++)
      InternalMediaFile(
        id: allTracks[i].id,
        libraryId: allTracks[i].libraryId,
        track: allTracks[i].track,
        path: allTracks[i].path,
        artist: allTracks[i].artist,
        album: allTracks[i].album,
        title: allTracks[i].title,
        duration: allTracks[i].duration,
        coverArtPath: allTracks[i].coverArtPath,
        trackNumber: allTracks[i].trackNumber,
        audioAvailability: allTracks[i].audioAvailability,
        audioOrigin: allTracks[i].audioOrigin,
        playlistOccurrenceId: 'occ-$i',
        playlistPosition: i,
      ),
  ];
  final next = end < allTracks.length
      ? PlaylistOccurrenceCursor(
          position: end - 1,
          occurrenceId: 'occ-${end - 1}',
        )
      : null;
  return PlaylistOccurrencePage(items, next);
}

Future<List<InternalMediaFile>> _serveMixPage(
  List<InternalMediaFile> allTracks,
  int? cursor,
  int? pageSize,
) async {
  final start = cursor ?? 0;
  final size = pageSize ?? 30;
  if (start >= allTracks.length) return const [];
  final end = (start + size).clamp(0, allTracks.length);
  return allTracks.sublist(start, end);
}

void main() {
  test('playlist with 1001 tracks exports every track in order', () async {
    final tracks = List.generate(1001, (i) => _track(i + 1));

    final m3u8 = await buildM3u8(
      CollectionType.playlist,
      7,
      loadQuery: (type, id) async => buildCollectionQuery(type, id),
      loadPlaylistPage: (playlistId, after, pageSize) {
        expect(playlistId, 7);
        return _servePlaylistPage(tracks, after, pageSize);
      },
      loadMixPage: (queries, [cursor, pageSize]) =>
          _serveMixPage(tracks, cursor, pageSize),
    );

    final lines = _m3u8Lines(m3u8);
    expect(lines.length, 1001);
    expect(lines, tracks.map((t) => t.path).toList());
  });

  test('playlist with repeated tracks exports every occurrence in order', () async {
    final a = _track(1);
    final b = _track(2);
    final c = _track(3);
    final occurrences = [a, b, a, c];

    final m3u8 = await buildM3u8(
      CollectionType.playlist,
      11,
      loadQuery: (type, id) async => buildCollectionQuery(type, id),
      loadPlaylistPage: (playlistId, after, pageSize) {
        expect(playlistId, 11);
        return _servePlaylistPage(occurrences, after, pageSize);
      },
      loadMixPage: (queries, [cursor, pageSize]) =>
          _serveMixPage(occurrences, cursor, pageSize),
    );

    final lines = _m3u8Lines(m3u8);
    expect(lines, [a.path, b.path, a.path, c.path]);
  });

  test('non-playlist collections page to completion with no fixed upper limit', () async {
    final tracks = List.generate(1001, (i) => _track(i + 1));

    for (final type in [
      CollectionType.album,
      CollectionType.artist,
      CollectionType.genre,
      CollectionType.mix,
    ]) {
      final m3u8 = await buildM3u8(
        type,
        42,
        loadQuery: (t, id) async => t == CollectionType.mix
            ? [('lib::artist', '42'), ('sort::duration', 'true')]
            : buildCollectionQuery(t, id),
        loadPlaylistPage: (playlistId, after, pageSize) {
          fail('non-playlist collection $type should not call loadPlaylistPage');
        },
        loadMixPage: (queries, [cursor, pageSize]) =>
            _serveMixPage(tracks, cursor, pageSize),
      );

      final lines = _m3u8Lines(m3u8);
      expect(lines.length, 1001, reason: 'collection type $type');
      expect(lines, tracks.map((t) => t.path).toList(), reason: 'collection type $type');
    }
  });

  test('collections whose length is an exact multiple of pageSize terminate cleanly', () async {
    final tracks = List.generate(200, (i) => _track(i + 1));

    final playlistM3u8 = await buildM3u8(
      CollectionType.playlist,
      3,
      pageSize: 100,
      loadQuery: (type, id) async => buildCollectionQuery(type, id),
      loadPlaylistPage: (playlistId, after, pageSize) =>
          _servePlaylistPage(tracks, after, pageSize),
      loadMixPage: (queries, [cursor, pageSize]) {
        fail('playlist should not call loadMixPage');
      },
    );
    expect(_m3u8Lines(playlistM3u8), tracks.map((t) => t.path).toList());

    final albumM3u8 = await buildM3u8(
      CollectionType.album,
      5,
      pageSize: 100,
      loadQuery: (type, id) async => buildCollectionQuery(type, id),
      loadPlaylistPage: (playlistId, after, pageSize) {
        fail('album should not call loadPlaylistPage');
      },
      loadMixPage: (queries, [cursor, pageSize]) =>
          _serveMixPage(tracks, cursor, pageSize),
    );
    expect(_m3u8Lines(albumM3u8), tracks.map((t) => t.path).toList());
  });

  test('empty collection exports an empty string', () async {
    final playlistM3u8 = await buildM3u8(
      CollectionType.playlist,
      1,
      loadQuery: (type, id) async => buildCollectionQuery(type, id),
      loadPlaylistPage: (playlistId, after, pageSize) async =>
          PlaylistOccurrencePage(const [], null),
      loadMixPage: (queries, [cursor, pageSize]) async => const [],
    );
    expect(playlistM3u8, isEmpty);

    final albumM3u8 = await buildM3u8(
      CollectionType.album,
      1,
      loadQuery: (type, id) async => buildCollectionQuery(type, id),
      loadPlaylistPage: (playlistId, after, pageSize) async =>
          PlaylistOccurrencePage(const [], null),
      loadMixPage: (queries, [cursor, pageSize]) async => const [],
    );
    expect(albumM3u8, isEmpty);
  });
}
