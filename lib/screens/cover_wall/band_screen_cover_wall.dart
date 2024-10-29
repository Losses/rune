import 'dart:async';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../config/animation.dart';
import '../../widgets/smooth_horizontal_scroll.dart';
import '../../widgets/start_screen/band_link_tile.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../providers/status.dart';
import '../../providers/volume.dart';
import '../../providers/playback_controller.dart';
import '../../providers/responsive_providers.dart';

class BandScreenCoverWallView extends StatefulWidget {
  const BandScreenCoverWallView({super.key});

  @override
  LibraryHomeListState createState() => LibraryHomeListState();
}

class LibraryHomeListState extends State<BandScreenCoverWallView> {
  final layoutManager = StartScreenLayoutManager();

  @override
  void dispose() {
    super.dispose();
    layoutManager.dispose();
  }

  @override
  void initState() {
    super.initState();

    Timer(
      Duration(milliseconds: gridAnimationDelay),
      () => layoutManager.playAnimations(),
    );
  }

  @override
  Widget build(BuildContext context) {
    final controllers = Provider.of<PlaybackControllerProvider>(context);
    Provider.of<PlaybackStatusProvider>(context);
    Provider.of<VolumeProvider>(context);

    final children = controllers.entries
        .where((x) => x.onShortcut != null)
        .map(
          (item) => Padding(
            padding: const EdgeInsets.symmetric(
              horizontal: 2,
              vertical: 1,
            ),
            child: AspectRatio(
              aspectRatio: 1,
              child: BandLinkTile(
                title: item.title,
                onPressed: () {
                  final fn = item.onShortcut;
                  if (fn != null) {
                    fn(context);
                  }
                },
                icon: item.icon(context),
              ),
            ),
          ),
        )
        .toList();

    return DeviceTypeBuilder(
      deviceType: const [DeviceType.band, DeviceType.dock, DeviceType.tv],
      builder: (context, deviceType) {
        if (deviceType == DeviceType.dock) {
          return SingleChildScrollView(
            child: Column(
              children: children,
            ),
          );
        }

        return SmoothHorizontalScroll(
          builder: (context, controller) {
            return SingleChildScrollView(
              controller: controller,
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
