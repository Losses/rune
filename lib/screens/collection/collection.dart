import 'package:fluent_ui/fluent_ui.dart';

import '../../widgets/belt_container.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../messages/collection.pb.dart';
import '../../providers/responsive_providers.dart';

import 'band_screen_collection_list.dart';
import 'small_screen_collection_list.dart';
import 'large_screen_collection_list.dart';

class CollectionPage extends StatelessWidget {
  final CollectionType collectionType;
  const CollectionPage({super.key, required this.collectionType});

  @override
  Widget build(BuildContext context) {
    return PageContentFrame(
      child: BreakpointBuilder(
        breakpoints: const [
          DeviceType.belt,
          DeviceType.dock,
          DeviceType.band,
          DeviceType.zune,
          DeviceType.tv
        ],
        builder: (context, activeBreakpoint) {
          if (activeBreakpoint == DeviceType.belt) {
            return BeltContainer(
              child: BandScreenCollectionListView(
                collectionType: collectionType,
              ),
            );
          }

          if (activeBreakpoint == DeviceType.dock ||
              activeBreakpoint == DeviceType.band) {
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
    );
  }
}
