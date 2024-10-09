import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:player/widgets/turntile/link_turntile.dart';

import '../../config/animation.dart';
import '../../widgets/turntile/turntile_group.dart';
import '../../screens/settings_home/constants/first_column.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';

class SmallScreenSettingsHomeListView extends StatefulWidget {
  final StartScreenLayoutManager layoutManager;

  const SmallScreenSettingsHomeListView(
      {super.key, required this.layoutManager});

  @override
  LibraryHomeListState createState() => LibraryHomeListState();
}

class LibraryHomeListState extends State<SmallScreenSettingsHomeListView> {
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
        child: TurntileGroup<(String, String, IconData)>(
          groupIndex: 0,
          items: firstColumn,
          gridLayoutVariation: TurntileGroupGridLayoutVariation.list,
          gapSize: 12,
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
