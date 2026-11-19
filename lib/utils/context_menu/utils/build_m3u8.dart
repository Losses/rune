import '../../../bindings/bindings.dart';
import '../../../widgets/track_list/utils/internal_media_file.dart';

import '../../api/fetch_playlist_occurrences.dart';
import '../../api/query_mix_tracks.dart';
import '../../build_query.dart';
import '../../get_playlist_id_from_query_list.dart';
import '../../query_list.dart';

/// Page size used when reading tracks for M3U8 export.
/// Must stay within `list_occurrences`'s `1..=128` page size bound.
const int m3u8ExportPageSize = 100;

/// Builds an M3U8 playlist string containing every track path in the
/// collection, paging until the collection ends.
Future<String> buildM3u8(
  CollectionType type,
  int id, {
  int pageSize = m3u8ExportPageSize,
  Future<List<(String, String)>> Function(CollectionType type, int id)
      loadQuery = buildQuery,
  Future<PlaylistOccurrencePage> Function(
    int playlistId,
    PlaylistOccurrenceCursor? after,
    int pageSize,
  )
      loadPlaylistPage = fetchPlaylistOccurrences,
  Future<List<InternalMediaFile>> Function(
    QueryList queries, [
    int? cursor,
    int? pageSize,
  ])
      loadMixPage = queryMixTracks,
}) async {
  if (pageSize <= 0) {
    throw ArgumentError.value(pageSize, 'pageSize', 'must be positive');
  }

  final queries = QueryList(await loadQuery(type, id));
  final items = await _loadAllTracks(
    queries,
    pageSize: pageSize,
    loadPlaylistPage: loadPlaylistPage,
    loadMixPage: loadMixPage,
  );

  return items
      .map((x) => x.path)
      .where((path) => path.isNotEmpty)
      .join('\r\n');
}

Future<List<InternalMediaFile>> _loadAllTracks(
  QueryList queries, {
  required int pageSize,
  required Future<PlaylistOccurrencePage> Function(
    int playlistId,
    PlaylistOccurrenceCursor? after,
    int pageSize,
  )
      loadPlaylistPage,
  required Future<List<InternalMediaFile>> Function(
    QueryList queries, [
    int? cursor,
    int? pageSize,
  ])
      loadMixPage,
}) async {
  final playlistId = getPlaylistIdFromQueryList(queries);
  if (playlistId != null) {
    final items = <InternalMediaFile>[];
    PlaylistOccurrenceCursor? cursor;
    while (true) {
      final page = await loadPlaylistPage(playlistId, cursor, pageSize);
      items.addAll(page.items);
      if (page.next == null) {
        return items;
      }
      cursor = page.next;
    }
  }

  final items = <InternalMediaFile>[];
  var cursor = 0;
  while (true) {
    final page = await loadMixPage(queries, cursor, pageSize);
    items.addAll(page);
    if (page.length < pageSize) {
      return items;
    }
    cursor += page.length;
  }
}
