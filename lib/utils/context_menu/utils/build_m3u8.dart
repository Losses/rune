import '../../../utils/query_list.dart';
import '../../../bindings/bindings.dart';

import '../../build_query.dart';
import '../../api/query_mix_tracks.dart';

Future<String> buildM3u8(
  CollectionType type,
  int id,
) async {
  final queries = await buildQuery(type, id);
  final newItems = await queryMixTracks(
    QueryList(queries),
    0,
    999,
  );

  final filePaths = newItems.map((x) => x.path).join('\r\n');

  return filePaths;
}
