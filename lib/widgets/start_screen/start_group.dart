import 'package:fluent_ui/fluent_ui.dart';

import './normal_layout.dart';
import './stacked_layout.dart';
import './start_group_items.dart';

enum StartGroupGridLayoutVariation { initial, square }

enum StartGroupGroupLayoutVariation { normal, stacked }

class StartGroup<T> extends StatelessWidget {
  final List<T> items;
  final String groupTitle;
  final int groupIndex;
  final Widget Function(BuildContext, T) itemBuilder;
  final StartGroupGridLayoutVariation gridLayoutVariation;
  final StartGroupGroupLayoutVariation groupLayoutVariation;

  final double gapSize;
  final VoidCallback? onTitleTap;

  const StartGroup({
    super.key,
    required this.groupIndex,
    required this.groupTitle,
    required this.items,
    required this.itemBuilder,
    this.gapSize = 4,
    this.onTitleTap,
    this.gridLayoutVariation = StartGroupGridLayoutVariation.initial,
    this.groupLayoutVariation = StartGroupGroupLayoutVariation.normal,
  });

  @override
  Widget build(BuildContext context) {
    return FocusTraversalGroup(
      child: _buildStartGroupLayout(
        context,
        _buildStartGroupItems(),
      ),
    );
  }

  Widget _buildStartGroupLayout(BuildContext context, Widget child) {
    switch (groupLayoutVariation) {
      case StartGroupGroupLayoutVariation.stacked:
        return StartGroupStackedLayout(
          groupTitle: groupTitle,
          onTitleTap: onTitleTap,
          child: child,
        );
      case StartGroupGroupLayoutVariation.normal:
      default:
        return StartGroupNormalLayout(
          groupTitle: groupTitle,
          onTitleTap: onTitleTap,
          child: child,
        );
    }
  }

  Widget _buildStartGroupItems() {
    switch (gridLayoutVariation) {
      case StartGroupGridLayoutVariation.square:
        return StartGroupItems<T>.square(
          cellSize: 120,
          gapSize: gapSize,
          items: items,
          groupIndex: groupIndex,
          itemBuilder: itemBuilder,
        );
      case StartGroupGridLayoutVariation.initial:
      default:
        return StartGroupItems<T>(
          cellSize: 120,
          gapSize: gapSize,
          items: items,
          groupIndex: groupIndex,
          itemBuilder: itemBuilder,
        );
    }
  }
}
