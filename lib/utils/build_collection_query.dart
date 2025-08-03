import '../utils/context_menu/collection_item_context_menu.dart';
import '../bindings/bindings.dart';

List<(String, String)> buildCollectionQuery(
  CollectionType collectionType,
  int id,
) {
  if (collectionType == CollectionType.mix) {
    throw "Not Allow";
  }
  return [(typeToOperator[collectionType]!, id.toString())];
}
