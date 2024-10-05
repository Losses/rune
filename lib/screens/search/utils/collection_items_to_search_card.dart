import '../../../widgets/start_screen/start_screen.dart';
import '../../../screens/search/widgets/collection_search_item.dart';
import '../../../messages/collection.pb.dart';

List<CollectionSearchItem> collectionItemsToSearchCard(
  List<InternalCollection> items,
  CollectionType collectionType,
) {
  return items
      .map(
        (a) => CollectionSearchItem(
          item: a,
          collectionType: collectionType,
        ),
      )
      .toList();
}
