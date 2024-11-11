import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import '../../config/animation.dart';

import '../start_screen/constants/default_gap_size.dart';
import '../start_screen/providers/start_screen_layout_manager.dart';
import '../navigation_bar/page_content_frame.dart';

import 'link_turntile.dart';
import 'turntile_group.dart';

class SmallScreenFeatureListView extends StatefulWidget {
  const SmallScreenFeatureListView({
    super.key,
    required this.layoutManager,
    required this.items,
    required this.topPadding,
  });
  final StartScreenLayoutManager layoutManager;
  final List<(String, String, IconData, bool)> items;
  final bool topPadding;

  @override
  LibraryHomeListState createState() => LibraryHomeListState();
}

class LibraryHomeListState extends State<SmallScreenFeatureListView> {
  @override
  void initState() {
    super.initState();

    Timer(
      Duration(milliseconds: gridAnimationDelay),
      () => widget.layoutManager.playAnimations(),
    );
  }

  @override
  Widget build(BuildContext context) {
    return Center(
      child: SingleChildScrollView(
        padding: getScrollContainerPadding(context, top: widget.topPadding),
        child: TurntileGroup<(String, String, IconData, bool)>(
          groupIndex: 0,
          items: widget.items,
          gridLayoutVariation: TurntileGroupGridLayoutVariation.list,
          gapSize: defaultGapSize,
          onTitleTap: () {},
          itemBuilder: (context, item) {
            return LinkTurntile(
              title: item.$1,
              path: item.$2,
            );
          },
        ),
      ),
    );
  }
}
