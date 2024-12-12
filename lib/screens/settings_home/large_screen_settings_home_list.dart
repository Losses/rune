import 'dart:async';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/material_symbols_icons.dart';

import '../../utils/l10n.dart';
import '../../utils/dialogs/register/show_register_dialog.dart';
import '../../config/animation.dart';
import '../../widgets/smooth_horizontal_scroll.dart';
import '../../widgets/start_screen/link_tile.dart';
import '../../widgets/start_screen/start_group.dart';
import '../../widgets/start_screen/constants/default_gap_size.dart';
import '../../widgets/start_screen/start_group_implementation.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../screens/settings_home/constants/first_column.dart';
import '../../providers/license.dart';

class LargeScreenSettingsHomeListView extends StatefulWidget {
  const LargeScreenSettingsHomeListView({
    super.key,
    required this.layoutManager,
    required this.topPadding,
    required this.mysterious,
  });

  final StartScreenLayoutManager layoutManager;
  final bool topPadding;
  final bool? mysterious;

  @override
  LargeScreenSettingsHomeListViewState createState() =>
      LargeScreenSettingsHomeListViewState();
}

class LargeScreenSettingsHomeListViewState
    extends State<LargeScreenSettingsHomeListView> {
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
    final license = Provider.of<LicenseProvider>(context);

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
                  if (!license.isStoreMode && !license.isPro)
                    StartGroup<
                        (
                          String Function(BuildContext),
                          void Function(),
                          IconData,
                          bool
                        )>(
                      groupIndex: 1,
                      groupTitle: "",
                      items: [
                        (
                          (context) => S.of(context).register,
                          () => showRegisterDialog(context),
                          Symbols.key,
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
                          onPressed: item.$2,
                          icon: item.$3,
                        );
                      },
                      direction: Axis.horizontal,
                    ),
                  if (widget.mysterious == true)
                    StartGroup<
                        (
                          String Function(BuildContext),
                          String,
                          IconData,
                          bool
                        )>(
                      groupIndex: 2,
                      groupTitle: "",
                      items: [
                        (
                          (context) => S.of(context).laboratory,
                          '/settings/laboratory',
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
