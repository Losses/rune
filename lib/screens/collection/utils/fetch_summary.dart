
import '../../../utils/api/fetch_collection_group_summary.dart';
import '../../../widgets/start_screen/utils/group.dart';
import '../../../widgets/start_screen/utils/internal_collection.dart';
import '../../../bindings/bindings.dart';

Future<List<Group<InternalCollection>>> fetchCollectionPageSummary(
  CollectionType collectionType,
) async {
  final groups = await fetchCollectionGroupSummary(collectionType);

  return groups.map((summary) {
    return Group<InternalCollection>(
      groupTitle: summary.groupTitle,
      items: [], // Initially empty, will be filled in fetchPage
    );
  }).toList();
}
