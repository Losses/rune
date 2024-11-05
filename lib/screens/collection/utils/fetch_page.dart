import '../../../widgets/start_screen/utils/group.dart';
import '../../../widgets/start_screen/utils/internal_collection.dart';
import '../../../messages/collection.pb.dart';

import 'fetch_summary.dart';
import 'fetch_groups.dart';

Future<(List<Group<InternalCollection>>, bool)> fetchCollectionPagePage(
  CollectionType collectionType,
  int pageSize,
  int cursor,
) async {
  final summaries = await fetchCollectionPageSummary(collectionType);

  final startIndex = cursor * pageSize;
  final endIndex = (cursor + 1) * pageSize;

  if (startIndex >= summaries.length) {
    final List<Group<InternalCollection>> result = [];
    return (result, true);
  }

  final currentPageSummaries = summaries.sublist(
    startIndex,
    endIndex > summaries.length ? summaries.length : endIndex,
  );

  final groupTitles =
      currentPageSummaries.map((summary) => summary.groupTitle).toList();
      
  final runeIndex = groupTitles.indexOf('\u{200B}Rune');

  if (runeIndex != -1) {
    groupTitles.removeAt(runeIndex);
    groupTitles.insert(0, '\u{200B}Rune');
  }

  final groups = await fetchCollectionPageGroups(collectionType, groupTitles);

  final isLastPage = endIndex >= summaries.length;

  return (groups, isLastPage);
}