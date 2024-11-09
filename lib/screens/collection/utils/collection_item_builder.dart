import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../../widgets/start_screen/utils/internal_collection.dart';

import '../collection_item.dart';

import 'collection_data_provider.dart';

Widget collectionItemBuilder(
  BuildContext context,
  InternalCollection item,
) {
  final data = Provider.of<CollectionDataProvider>(context);

  return CollectionItem(
    collection: item,
    collectionType: item.collectionType,
    refreshList: data.reloadData,
  );
}
