import 'package:fluent_ui/fluent_ui.dart';
import 'package:player/messages/collection.pb.dart';

import '../../widgets/playback_controller/playback_placeholder.dart';
import '../../widgets/navigation_bar/navigation_bar_placeholder.dart';

import 'collection_list.dart';

class CollectionPage extends StatelessWidget {
  final CollectionType collectionType;
  const CollectionPage({super.key, required this.collectionType});

  @override
  Widget build(BuildContext context) {
    return Column(children: [
      const NavigationBarPlaceholder(),
      Expanded(
        child: CollectionListView(
          collectionType: collectionType,
        ),
      ),
      const PlaybackPlaceholder()
    ]);
  }
}
