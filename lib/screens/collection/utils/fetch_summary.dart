import '../../../utils/api/fetch_collection_group_summary.dart';
import '../../../widgets/start_screen/utils/group.dart';
import '../../../widgets/start_screen/utils/internal_collection.dart';
import '../../../bindings/bindings.dart';

Future<List<Group<InternalCollection>>> fetchCollectionPageSummary(
  CollectionType collectionType,
) async {
  final summary = await fetchCollectionGroupSummary(collectionType);
  final reserved = summary.reservedGroups.toSet();

  return summary.groups.map((group) {
    return Group<InternalCollection>(
      groupTitle: group.groupTitle,
      items: [], // Initially empty, will be filled in fetchPage
      reserved: reserved.contains(group.groupTitle),
    );
  }).toList();
}
