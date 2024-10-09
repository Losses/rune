import 'package:fluent_ui/fluent_ui.dart';

import '../../screens/collection/large_screen_collection_list.dart';
import '../../screens/collection/small_screen_collection_list.dart';
import '../../widgets/playback_controller/playback_placeholder.dart';
import '../../widgets/navigation_bar/navigation_bar_placeholder.dart';
import '../../messages/collection.pb.dart';
import '../../providers/responsive_providers.dart';

class CollectionPage extends StatelessWidget {
  final CollectionType collectionType;
  const CollectionPage({super.key, required this.collectionType});

  @override
  Widget build(BuildContext context) {
    return Column(children: [
      const NavigationBarPlaceholder(),
      Expanded(
        child: BreakpointBuilder(
          breakpoints: const [DeviceType.zune, DeviceType.tv],
          builder: (context, activeBreakpoint) {
            return activeBreakpoint == DeviceType.zune
                ? SmallScreenCollectionListView(
                    collectionType: collectionType,
                  )
                : LargeScreenCollectionListView(
                    collectionType: collectionType,
                  );
          },
        ),
      ),
      const PlaybackPlaceholder()
    ]);
  }
}
