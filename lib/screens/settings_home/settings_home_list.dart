import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../config/animation.dart';
import '../../widgets/smooth_horizontal_scroll.dart';
import '../../widgets/start_screen/link_tile.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../widgets/start_screen/start_group.dart';

class SettingsHomeList extends StatefulWidget {
  final StartScreenLayoutManager layoutManager;

  const SettingsHomeList({super.key, required this.layoutManager});

  @override
  SettingsHomeListState createState() => SettingsHomeListState();
}

class SettingsHomeListState extends State<SettingsHomeList> {
  final _layoutManager = StartScreenLayoutManager();

  @override
  void initState() {
    super.initState();
    Timer(
      Duration(milliseconds: gridAnimationDelay),
      () => widget.layoutManager.playAnimations(),
    );
  }

  @override
  void dispose() {
    super.dispose();
    _layoutManager.dispose();
  }

  final List<(String, String, IconData)> firstColumn = [
    ('Library', '/settings/library', Symbols.video_library),
    ('Controller', '/settings/media_conrtoller', Symbols.tune),
    ('About', '/settings/about', Symbols.info),
  ];

  @override
  Widget build(BuildContext context) {
    return Container(
      alignment: Alignment.centerLeft,
      child: SmoothHorizontalScroll(
        builder: (context, scrollController) => SingleChildScrollView(
          scrollDirection: Axis.horizontal,
          controller: scrollController,
          child: Row(
            mainAxisAlignment: MainAxisAlignment.start,
            children: [
              StartGroup<(String, String, IconData)>(
                groupIndex: 0,
                groupTitle: 'Explore',
                items: firstColumn,
                groupLayoutVariation: StartGroupGroupLayoutVariation.stacked,
                gridLayoutVariation: StartGroupGridLayoutVariation.initial,
                gapSize: 12,
                onTitleTap: () {},
                itemBuilder: (context, item) {
                  return LinkTile(
                    title: item.$1,
                    path: item.$2,
                    icon: item.$3,
                  );
                },
              ),
            ],
          ),
        ),
      ),
    );
  }
}
