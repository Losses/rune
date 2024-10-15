import 'package:fluent_ui/fluent_ui.dart';
import 'package:rune/screens/collection/band_screen_collection_list.dart';

import '../../screens/collection/small_screen_collection_list.dart';
import '../../widgets/playback_controller/controllor_placeholder.dart';
import '../../widgets/navigation_bar/navigation_bar_placeholder.dart';
import '../../messages/collection.pb.dart';
import '../../providers/responsive_providers.dart';

import 'large_screen_collection_list.dart';

class CollectionPage extends StatelessWidget {
  final CollectionType collectionType;
  const CollectionPage({super.key, required this.collectionType});

  @override
  Widget build(BuildContext context) {
    return Column(children: [
      const NavigationBarPlaceholder(),
      Expanded(
        child: BreakpointBuilder(
          breakpoints: const [DeviceType.band, DeviceType.zune, DeviceType.tv],
          builder: (context, activeBreakpoint) {
            if (activeBreakpoint == DeviceType.band) {
              return BandScreenCollectionListView(
                collectionType: collectionType,
              );
            }

            if (activeBreakpoint == DeviceType.zune) {
              return SmallScreenCollectionListView(
                collectionType: collectionType,
              );
            }

            return LargeScreenCollectionListView(
              collectionType: collectionType,
            );
          },
        ),
      ),
      const ControllerPlaceholder()
    ]);
  }
}
