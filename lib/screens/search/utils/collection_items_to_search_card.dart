import '../../../widgets/start_screen/utils/internal_collection.dart';
import '../../../screens/search/widgets/collection_search_item.dart';
import '../../../bindings/bindings.dart';

List<CollectionSearchItem> collectionItemsToSearchCard(
  List<InternalCollection> items,
  CollectionType collectionType,
) {
  return items
      .map(
        (a) => CollectionSearchItem(
          index: 0,
          item: a,
          collectionType: collectionType,
        ),
      )
      .toList();
}
