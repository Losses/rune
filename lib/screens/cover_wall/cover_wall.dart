import 'package:fluent_ui/fluent_ui.dart';

import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../providers/responsive_providers.dart';

import 'widgets/cover_wall_layout.dart';
import 'band_screen_cover_wall.dart';

class CoverWallPage extends StatefulWidget {
  const CoverWallPage({super.key});

  @override
  State<CoverWallPage> createState() => _CoverWallPageState();
}

class _CoverWallPageState extends State<CoverWallPage> {
  @override
  Widget build(BuildContext context) {
    return DeviceTypeBuilder(
      deviceType: const [
        DeviceType.band,
        DeviceType.dock,
        DeviceType.zune,
        DeviceType.tv
      ],
      builder: (context, activeBreakpoint) {
        if (activeBreakpoint == DeviceType.dock ||
            activeBreakpoint == DeviceType.band) {
          return const PageContentFrame(child: BandScreenCoverWallView());
        }

        return const CoverWallLayout();
      },
    );
  }
}
