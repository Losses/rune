import '../bindings/bindings.dart';
import 'api/fetch_mix_queries_by_mix_id.dart';
import 'build_collection_query.dart';

Future<List<(String, String)>> buildQuery(
  CollectionType type,
  int id,
) async {
  return type == CollectionType.mix
      ? await fetchMixQueriesByMixId(id)
      : buildCollectionQuery(type, id);
}

List<(String, String)> withRecommend(List<(String, String)> x) {
  return [
    ...x,
    ("pipe::limit", "50"),
    ("pipe::recommend", "-1"),
  ];
}
