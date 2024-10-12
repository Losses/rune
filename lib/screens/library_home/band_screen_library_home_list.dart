import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';

import '../../config/animation.dart';
import '../../screens/library_home/constants/first_column.dart';
import '../../widgets/start_screen/band_link_tile.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';

class BandScreenLibraryHomeListView extends StatefulWidget {
  final StartScreenLayoutManager layoutManager;

  const BandScreenLibraryHomeListView({super.key, required this.layoutManager});

  @override
  LibraryHomeListState createState() => LibraryHomeListState();
}

class LibraryHomeListState extends State<BandScreenLibraryHomeListView> {
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
    return SingleChildScrollView(
      child: Column(
        children: firstColumn
            .map(
              (item) => Padding(
                padding: const EdgeInsets.symmetric(
                  horizontal: 2,
                  vertical: 1,
                ),
                child: AspectRatio(
                  aspectRatio: 1,
                  child: BandLinkTile(
                    title: item.$1,
                    path: item.$2,
                    icon: item.$3,
                  ),
                ),
              ),
            )
            .toList(),
      ),
    );
  }
}
