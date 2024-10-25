import 'package:fluent_ui/fluent_ui.dart';

import '../../screens/cover_wall/band_screen_cover_wall.dart';
import '../../providers/responsive_providers.dart';

import 'large_screen_cover_wall.dart';

class CoverWallPage extends StatefulWidget {
  const CoverWallPage({super.key});

  @override
  State<CoverWallPage> createState() => _CoverWallPageState();
}

class _CoverWallPageState extends State<CoverWallPage> {
  @override
  Widget build(BuildContext context) {
    return SmallerOrEqualTo(
      deviceType: DeviceType.dock,
      builder: (context, isDock) {
        if (isDock) {
          return const BandScreenCoverWallView();
        }

        return const LargeScreenCoverWallView();
      },
    );
  }
}
