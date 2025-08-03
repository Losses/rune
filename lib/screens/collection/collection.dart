import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../widgets/belt_container.dart';
import '../../widgets/turntile/turntile_screen.dart';
import '../../widgets/band_screen/band_screen.dart';
import '../../widgets/start_screen/start_screen.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../bindings/bindings.dart';
import '../../providers/responsive_providers.dart';

import 'utils/collection_data_provider.dart';

class CollectionPage extends StatefulWidget {
  final CollectionType collectionType;
  const CollectionPage({super.key, required this.collectionType});

  @override
  State<CollectionPage> createState() => _CollectionPageState();
}

class _CollectionPageState extends State<CollectionPage> {
  late CollectionDataProvider collectionData =
      CollectionDataProvider(collectionType: widget.collectionType);

  @override
  Widget build(BuildContext context) {
    return ChangeNotifierProvider.value(
      value: collectionData,
      child: PageContentFrame(
        child: DeviceTypeBuilder(
          deviceType: const [
            DeviceType.band,
            DeviceType.belt,
            DeviceType.dock,
            DeviceType.zune,
            DeviceType.tv
          ],
          builder: (context, activeBreakpoint) {
            if (activeBreakpoint == DeviceType.belt) {
              return const BeltContainer(
                child: BandScreen(),
              );
            }

            if (activeBreakpoint == DeviceType.dock ||
                activeBreakpoint == DeviceType.band) {
              return const BandScreen();
            }

            if (activeBreakpoint == DeviceType.zune) {
              return const TurntileScreen();
            }

            return const StartScreen();
          },
        ),
      ),
    );
  }
}
