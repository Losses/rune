import 'package:fluent_ui/fluent_ui.dart';

import '../../widgets/turntile/turntile_screen.dart';
import '../../screens/collection/base_collection_list_view.dart';
import '../../screens/collection/utils/is_user_generated.dart';

class SmallScreenCollectionListView extends BaseCollectionListView {
  const SmallScreenCollectionListView(
      {super.key, required super.collectionType});

  @override
  BaseCollectionListViewState createState() =>
      SmallScreenCollectionListViewState();
}

class SmallScreenCollectionListViewState
    extends BaseCollectionListViewState<SmallScreenCollectionListView> {
  @override
  Widget buildScreen(BuildContext context) {
    return TurntileScreen(
      fetchSummary: fetchSummary,
      fetchPage: fetchPage,
      itemBuilder: itemBuilder,
      userGenerated: userGenerated(widget.collectionType),
    );
  }
}
