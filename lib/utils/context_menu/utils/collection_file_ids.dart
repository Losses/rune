import 'dart:math';

import '../../../utils/query_list.dart';
import '../../../bindings/bindings.dart';

import '../../build_query.dart';
import '../../api/query_mix_tracks.dart';
import '../../../widgets/track_list/utils/internal_media_file.dart';

const _pageSize = 999;

/// The ids of the songs of a collection, in its order: the track order for an album or a genre,
/// as its page lists them. Only an empty page ends the list: a library may cap its pages below the
/// size asked for, as a Subsonic server does.
Future<List<int>> collectionFileIds(
  CollectionType type,
  int id, {
  Future<List<(String, String)>> Function(CollectionType type, int id)
      loadQueries = buildQuery,
  Future<List<InternalMediaFile>> Function(
    QueryList queries,
    int cursor,
    int pageSize,
    int randomSeed,
  )
      loadPage = _queryPage,
}) async {
  final queries = QueryList([
    ...await loadQueries(type, id),
    if (type == CollectionType.album || type == CollectionType.genre)
      ('sort::track_number', 'true'),
  ]);
  // One seed for every page, so that a random mix is one draw.
  final randomSeed = Random().nextInt(0x7fffffff);
  final ids = <int>[];
  while (true) {
    final page = await loadPage(queries, ids.length, _pageSize, randomSeed);
    if (page.isEmpty) return ids;
    ids.addAll(page.map((x) => x.id));
  }
}

Future<List<InternalMediaFile>> _queryPage(
  QueryList queries,
  int cursor,
  int pageSize,
  int randomSeed,
) => queryMixTracks(queries, cursor, pageSize, randomSeed);
