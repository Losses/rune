import 'package:fluent_ui/fluent_ui.dart';

import '../../../widgets/start_screen/utils/group.dart';
import '../../../widgets/start_screen/utils/internal_collection.dart';
import '../../../messages/all.dart';

import 'fetch_page.dart';
import 'fetch_groups.dart';
import 'fetch_summary.dart';

class CollectionDataProvider with ChangeNotifier {
  static const _pageSize = 3;
  final CollectionType collectionType;

  CollectionDataProvider({required this.collectionType});

  late Future<List<Group<InternalCollection>>> summary = fetchSummary();

  List<Group<InternalCollection>> items = [];

  bool isLoading = false;
  bool isLastPage = false;
  bool initialized = false;
  int cursor = 0;

  Future<List<Group<InternalCollection>>> fetchSummary() {
    return fetchCollectionPageSummary(collectionType);
  }

  Future<(List<Group<InternalCollection>>, bool)> _fetchPage(
    int cursor,
  ) async {
    return fetchCollectionPagePage(collectionType, _pageSize, cursor);
  }

  Future<List<Group<InternalCollection>>> fetchGroups(
    List<String> groupTitles,
  ) {
    return fetchCollectionPageGroups(collectionType, groupTitles);
  }

  Future<void> fetchData() async {
    initialized = true;
    isLoading = true;

    notifyListeners();

    final thisCursor = cursor;
    cursor += 1;
    final (newItems, newIsLastPage) = await _fetchPage(thisCursor);

    isLoading = false;
    isLastPage = newIsLastPage;
    items.addAll(newItems);

    notifyListeners();
  }

  void reloadData() async {
    cursor = 0;
    items = [];
    fetchData();
  }
}
