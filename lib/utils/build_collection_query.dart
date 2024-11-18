import '../utils/context_menu/collection_item_context_menu.dart';
import '../messages/collection.pb.dart';

List<(String, String)> buildCollectionQuery(
  CollectionType collectionType,
  int id,
) {
  if (collectionType == CollectionType.Mix) {
    throw "Not Allow";
  }
  return [(typeToOperator[collectionType]!, id.toString())];
}
