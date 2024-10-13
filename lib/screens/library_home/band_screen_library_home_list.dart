import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';

import '../../config/animation.dart';
import '../../widgets/band_screen/band_link_tile_list.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';

import 'constants/first_column.dart';

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
    return const BandLinkTileList(links: bandScreenFirstColumn);
  }
}
