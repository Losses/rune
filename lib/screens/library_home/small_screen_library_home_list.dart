import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';

import '../../config/animation.dart';
import '../../screens/library_home/constants/first_column.dart';
import '../../widgets/start_screen/constants/default_gap_size.dart';
import '../../widgets/turntile/link_turntile.dart';
import '../../widgets/turntile/turntile_group.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';

class SmallScreenLibraryHomeListView extends StatefulWidget {
  final StartScreenLayoutManager layoutManager;

  const SmallScreenLibraryHomeListView(
      {super.key, required this.layoutManager});

  @override
  LibraryHomeListState createState() => LibraryHomeListState();
}

class LibraryHomeListState extends State<SmallScreenLibraryHomeListView> {
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
        child: TurntileGroup<(String, String, IconData, bool)>(
          groupIndex: 0,
          items: smallScreenFirstColumn,
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
