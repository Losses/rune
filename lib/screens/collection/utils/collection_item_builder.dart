import 'package:fluent_ui/fluent_ui.dart';

import '../../../widgets/start_screen/utils/internal_collection.dart';
import '../collection_item.dart';

Widget collectionItemBuilder(
  BuildContext context,
  InternalCollection item,
  VoidCallback refreshList,
) {
  return CollectionItem(
    collection: item,
    collectionType: item.collectionType,
    refreshList: refreshList,
  );
}
