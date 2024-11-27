import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/material_symbols_icons.dart';

import '../../config/animation.dart';
import '../../utils/settings_manager.dart';
import '../../widgets/smooth_horizontal_scroll.dart';
import '../../widgets/start_screen/link_tile.dart';
import '../../widgets/start_screen/start_group.dart';
import '../../widgets/start_screen/constants/default_gap_size.dart';
import '../../widgets/start_screen/start_group_implementation.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../screens/settings_home/constants/first_column.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../utils/l10n.dart';
import '../settings_about/settings_about.dart';

class LargeScreenSettingsHomeListView extends StatefulWidget {
  const LargeScreenSettingsHomeListView({
    super.key,
    required this.layoutManager,
    required this.topPadding,
  });

  final StartScreenLayoutManager layoutManager;
  final bool topPadding;

  @override
  LargeScreenSettingsHomeListViewState createState() =>
      LargeScreenSettingsHomeListViewState();
}

class LargeScreenSettingsHomeListViewState
    extends State<LargeScreenSettingsHomeListView> {
  bool? mysterious;

  @override
  void initState() {
    super.initState();
    Timer(
      Duration(milliseconds: gridAnimationDelay),
      () => widget.layoutManager.playAnimations(),
    );

    SettingsManager().getValue<bool>(mysteriousKey).then((x) {
      if (!context.mounted) return;

      setState(() {
        mysterious = x;
      });
    });
  }

  @override
  Widget build(BuildContext context) {
    return Container(
      alignment: Alignment.centerLeft,
      child: SmoothHorizontalScroll(
        builder: (context, scrollController) => SingleChildScrollView(
          padding: getScrollContainerPadding(context, top: widget.topPadding),
          scrollDirection: Axis.horizontal,
          controller: scrollController,
          child: LayoutBuilder(
            builder: (context, constraints) {
              return Row(
                mainAxisAlignment: MainAxisAlignment.start,
                children: [
                  StartGroup<
                      (String Function(BuildContext), String, IconData, bool)>(
                    groupIndex: 0,
                    groupTitle: S.of(context).explore,
                    items: firstColumn,
                    constraints: constraints,
                    groupLayoutVariation:
                        StartGroupGroupLayoutVariation.stacked,
                    gridLayoutVariation: StartGroupGridLayoutVariation.initial,
                    dimensionCalculator:
                        StartGroupImplementation.startLinkDimensionCalculator,
                    gapSize: defaultGapSize,
                    itemBuilder: (context, item) {
                      return LinkTile(
                        title: item.$1(context),
                        path: item.$2,
                        icon: item.$3,
                      );
                    },
                    direction: Axis.horizontal,
                  ),
                  if (mysterious == true)
                    StartGroup<
                        (
                          String Function(BuildContext),
                          String,
                          IconData,
                          bool
                        )>(
                      groupIndex: 1,
                      groupTitle: S.of(context).mysteriousModalTitle,
                      items: [
                        (
                          (context) => S.of(context).laboratory,
                          '/settings/library',
                          Symbols.interests,
                          false
                        ),
                      ],
                      constraints: constraints,
                      groupLayoutVariation:
                          StartGroupGroupLayoutVariation.stacked,
                      gridLayoutVariation:
                          StartGroupGridLayoutVariation.initial,
                      dimensionCalculator:
                          StartGroupImplementation.startLinkDimensionCalculator,
                      gapSize: defaultGapSize,
                      itemBuilder: (context, item) {
                        return LinkTile(
                          title: item.$1(context),
                          path: item.$2,
                          icon: item.$3,
                        );
                      },
                      direction: Axis.horizontal,
                    ),
                ],
              );
            },
          ),
        ),
      ),
    );
  }
}
