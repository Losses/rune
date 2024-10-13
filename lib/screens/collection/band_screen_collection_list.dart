import 'package:fluent_ui/fluent_ui.dart';

import '../../widgets/band_screen/band_screen.dart';
import '../../screens/collection/base_collection_list_view.dart';
import '../../screens/collection/utils/is_user_generated.dart';

class BandScreenCollectionListView extends BaseCollectionListView {
  const BandScreenCollectionListView(
      {super.key, required super.collectionType});

  @override
  BaseCollectionListViewState createState() =>
      BandScreenCollectionListViewState();
}

class BandScreenCollectionListViewState
    extends BaseCollectionListViewState<BandScreenCollectionListView> {
  @override
  Widget buildScreen(BuildContext context) {
    return BandScreen(
      fetchSummary: fetchSummary,
      fetchPage: fetchPage,
      itemBuilder: itemBuilder,
      userGenerated: userGenerated(widget.collectionType),
    );
  }
}
