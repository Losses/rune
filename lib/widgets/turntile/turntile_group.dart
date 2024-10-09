import 'package:fluent_ui/fluent_ui.dart';

import 'turntile_normal_layout.dart';
import 'turntile_group_items_list.dart';
import 'turntile_group_items_tile.dart';



enum TurntileGroupGridLayoutVariation { list, tile }

class TurntileGroup<T> extends StatelessWidget {
  final List<T> items;
  final String? groupTitle;
  final int groupIndex;
  final Widget Function(BuildContext, T) itemBuilder;
  final TurntileGroupGridLayoutVariation gridLayoutVariation;

  final double gapSize;
  final VoidCallback? onTitleTap;

  const TurntileGroup({
    super.key,
    required this.groupIndex,
    this.groupTitle,
    required this.items,
    required this.itemBuilder,
    this.gapSize = 4,
    this.onTitleTap,
    this.gridLayoutVariation = TurntileGroupGridLayoutVariation.list,
  });

  @override
  Widget build(BuildContext context) {
    return _buildStartGroupLayout(context, _buildStartGroupItems());
  }

  Widget _buildStartGroupLayout(BuildContext context, Widget child) {
    return TurnTileGroupNormalLayout(
      groupTitle: groupTitle,
      onTitleTap: onTitleTap,
      child: child,
    );
  }

  Widget _buildStartGroupItems() {
    switch (gridLayoutVariation) {
      case TurntileGroupGridLayoutVariation.list:
        return TurntileGroupItemsList<T>(
          cellSize: 40,
          gapSize: gapSize,
          items: items,
          groupIndex: groupIndex,
          itemBuilder: itemBuilder,
        );
      case TurntileGroupGridLayoutVariation.tile:
      default:
        return TurntileGroupItemsTile<T>(
          cellSize: 88,
          gapSize: gapSize,
          items: items,
          groupIndex: groupIndex,
          itemBuilder: itemBuilder,
        );
    }
  }
}
