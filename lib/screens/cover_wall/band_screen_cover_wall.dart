import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:player/widgets/navigation_bar/navigation_bar_placeholder.dart';
import 'package:player/widgets/playback_controller/playback_placeholder.dart';
import 'package:provider/provider.dart';

import '../../config/animation.dart';
import '../../widgets/start_screen/band_link_tile.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../providers/playback_controller.dart';

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
    return Column(
      children: [
        const NavigationBarPlaceholder(),
        Expanded(
          child: SingleChildScrollView(
            child: Column(
              children: controllers.entries
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
                          icon: item.icon,
                        ),
                      ),
                    ),
                  )
                  .toList(),
            ),
          ),
        ),
        const PlaybackPlaceholder(),
      ],
    );
  }
}
