import 'dart:math';

import '../../../bindings/bindings.dart';
import '../../../widgets/track_list/utils/internal_media_file.dart';

import '../../api/fetch_playlist_occurrences.dart';
import '../../api/query_mix_tracks.dart';
import '../../build_query.dart';
import '../../query_list.dart';

/// Page size used when reading tracks for M3U8 export.
/// Must stay within `list_occurrences`'s `1..=128` page size bound.
const int m3u8ExportPageSize = 100;

/// Builds an M3U8 playlist string containing every track path in the
/// collection, paging until the collection ends.
Future<String> buildM3u8(
  CollectionType type,
  int id,
) async {
  final items = type == CollectionType.playlist
      ? await _loadAllPlaylistTracks(id)
      : await _loadAllMixTracks(QueryList(await buildQuery(type, id)));

  return items
      .map((x) => x.path)
      .where((path) => path.isNotEmpty)
      .join('\r\n');
}

Future<List<InternalMediaFile>> _loadAllPlaylistTracks(int playlistId) async {
  final items = <InternalMediaFile>[];
  PlaylistOccurrenceCursor? cursor;
  while (true) {
    final page = await fetchPlaylistOccurrences(
      playlistId,
      cursor,
      m3u8ExportPageSize,
    );
    items.addAll(page.items);
    if (page.next == null) {
      return items;
    }
    cursor = page.next;
  }
}

Future<List<InternalMediaFile>> _loadAllMixTracks(QueryList queries) async {
  final randomSeed = Random().nextInt(0x7fffffff);
  final items = <InternalMediaFile>[];
  var cursor = 0;
  while (true) {
    final page = await queryMixTracks(
      queries,
      cursor,
      m3u8ExportPageSize,
      randomSeed,
    );
    items.addAll(page);
    if (page.length < m3u8ExportPageSize) {
      return items;
    }
    cursor += page.length;
  }
}
