import 'package:rune/messages/collection.pbserver.dart';
import 'package:rune/utils/query_list.dart';

class InternalCollection {
  final int id;
  final String name;
  final QueryList queries;
  final CollectionType collectionType;
  final Map<int, String> coverArtMap;
  final bool readonly;

  InternalCollection({
    required this.id,
    required this.name,
    required this.queries,
    required this.collectionType,
    required this.coverArtMap,
    required this.readonly,
  });

  static InternalCollection fromRawCollection(Collection x) {
    return InternalCollection(
      id: x.id,
      name: x.name,
      queries: QueryList.fromMixQuery(x.queries),
      collectionType: x.collectionType,
      coverArtMap: x.coverArtMap,
      readonly: x.readonly,
    );
  }
}
