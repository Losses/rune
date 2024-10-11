import 'package:fluent_ui/fluent_ui.dart';
import 'package:player/messages/collection.pb.dart';

import '../../widgets/start_screen/utils/group.dart';
import '../../widgets/start_screen/utils/internal_collection.dart';

import 'utils/fetch_page.dart';
import 'utils/fetch_summary.dart';
import 'utils/fetch_groups.dart';

import 'collection_item.dart';

abstract class BaseCollectionListView extends StatefulWidget {
  final CollectionType collectionType;

  const BaseCollectionListView({super.key, required this.collectionType});

  @override
  BaseCollectionListViewState createState();
}

abstract class BaseCollectionListViewState<T extends BaseCollectionListView>
    extends State<T> {
  static const _pageSize = 3;

  Future<List<Group<InternalCollection>>> fetchSummary() {
    return fetchCollectionPageSummary(widget.collectionType);
  }

  Future<(List<Group<InternalCollection>>, bool)> fetchPage(
    int cursor,
  ) async {
    return fetchCollectionPagePage(widget.collectionType, _pageSize, cursor);
  }

  Future<List<Group<InternalCollection>>> fetchGroups(
    List<String> groupTitles,
  ) {
    return fetchCollectionPageGroups(widget.collectionType, groupTitles);
  }

  Widget itemBuilder(
    BuildContext context,
    InternalCollection item,
    VoidCallback refreshList,
  ) {
    return CollectionItem(
      collection: item,
      collectionType: widget.collectionType,
      refreshList: refreshList,
    );
  }

  Widget buildScreen(BuildContext context);

  @override
  Widget build(BuildContext context) {
    return buildScreen(context);
  }
}
