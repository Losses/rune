import 'dart:math';

import '../../../utils/query_list.dart';
import '../../../bindings/bindings.dart';

import '../../build_query.dart';
import '../../api/query_mix_tracks.dart';

const _pageSize = 999;

/// The ids of the songs of a collection, in its order. Only an empty page ends the list: a
/// library may cap its pages below the size asked for, as a Subsonic server does.
Future<List<int>> collectionFileIds(CollectionType type, int id) async {
  final queries = QueryList(await buildQuery(type, id));
  // One seed for every page, so that a random mix is one draw.
  final randomSeed = Random().nextInt(0x7fffffff);
  final ids = <int>[];
  while (true) {
    final page = await queryMixTracks(queries, ids.length, _pageSize, randomSeed);
    if (page.isEmpty) return ids;
    ids.addAll(page.map((x) => x.id));
  }
}
