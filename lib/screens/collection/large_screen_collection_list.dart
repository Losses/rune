import 'package:fluent_ui/fluent_ui.dart';

import '../../screens/collection/utils/is_user_generated.dart';
import '../../screens/collection/base_collection_list_view.dart';
import '../../widgets/start_screen/start_screen.dart';

class LargeScreenCollectionListView extends BaseCollectionListView {
  const LargeScreenCollectionListView(
      {super.key, required super.collectionType});

  @override
  BaseCollectionListViewState createState() =>
      LargeScreenCollectionListViewState();
}

class LargeScreenCollectionListViewState
    extends BaseCollectionListViewState<LargeScreenCollectionListView> {
  @override
  Widget buildScreen(BuildContext context) {
    return StartScreen(
      fetchSummary: fetchSummary,
      fetchPage: fetchPage,
      itemBuilder: itemBuilder,
      userGenerated: userGenerated(widget.collectionType),
    );
  }
}
