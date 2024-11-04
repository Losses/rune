import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';

import '../../config/animation.dart';
import 'band_link_tile_list.dart';
import '../start_screen/providers/start_screen_layout_manager.dart';

class BandScreenFeatureList extends StatefulWidget {
  const BandScreenFeatureList({
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

class LibraryHomeListState extends State<BandScreenFeatureList> {
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
    return BandLinkTileList(
      links: widget.items,
      topPadding: widget.topPadding,
    );
  }
}
