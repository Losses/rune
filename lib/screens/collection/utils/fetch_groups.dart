import '../../../utils/api/fetch_collection_groups.dart';
import '../../../widgets/start_screen/utils/group.dart';
import '../../../widgets/start_screen/utils/internal_collection.dart';
import '../../../bindings/bindings.dart';

Future<List<Group<InternalCollection>>> fetchCollectionPageGroups(
  CollectionType collectionType,
  List<String> groupTitles,
) async {
  final groups = await fetchCollectionGroups(collectionType, groupTitles);

  return groups.map((group) {
    return Group<InternalCollection>(
      groupTitle: group.groupTitle,
      items:
          group.collections.map(InternalCollection.fromRawCollection).toList(),
    );
  }).toList();
}
