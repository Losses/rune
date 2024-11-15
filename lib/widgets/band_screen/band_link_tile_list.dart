import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/router/navigation.dart';
import '../../widgets/smooth_horizontal_scroll.dart';
import '../../widgets/start_screen/band_link_tile.dart';
import '../../providers/responsive_providers.dart';

import '../navigation_bar/page_content_frame.dart';

class BandLinkTileList extends StatelessWidget {
  const BandLinkTileList({
    super.key,
    required this.links,
    required this.topPadding,
    this.leadingItem,
  });

  final List<(String Function(BuildContext), String, IconData, bool)> links;
  final bool topPadding;
  final Widget? leadingItem;

  @override
  Widget build(BuildContext context) {
    List<Widget> children = links
        .map(
          (item) => Padding(
            padding: const EdgeInsets.symmetric(
              horizontal: 2,
              vertical: 1,
            ),
            child: AspectRatio(
              aspectRatio: 1,
              child: BandLinkTile(
                title: item.$1(context),
                onPressed: () {
                  $push(item.$2);
                },
                icon: item.$3,
              ),
            ),
          ),
        )
        .toList();

    if (leadingItem != null) {
      children = [leadingItem!, ...children];
    }

    return DeviceTypeBuilder(
      deviceType: const [DeviceType.band, DeviceType.dock, DeviceType.tv],
      builder: (context, deviceType) {
        if (deviceType == DeviceType.dock) {
          return SingleChildScrollView(
            padding: getScrollContainerPadding(context, top: topPadding),
            child: Column(
              children: children,
            ),
          );
        }

        return SmoothHorizontalScroll(
          builder: (context, controller) {
            return SingleChildScrollView(
              controller: controller,
              padding: getScrollContainerPadding(context, top: topPadding),
              scrollDirection: Axis.horizontal,
              child: Row(
                children: children,
              ),
            );
          },
        );
      },
    );
  }
}
