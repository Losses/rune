import 'package:fluent_ui/fluent_ui.dart';

import '../tile_group/tile_grid.dart';
import '../tile_group/tile_layout.dart';

import 'managed_turntile_screen_item.dart';

class TurntileGroupItemTile<T> extends StatelessWidget {
  const TurntileGroupItemTile({
    super.key,
    required this.layout,
    required this.spans,
    required this.size,
    required this.cellSize,
    required this.gapSize,
    required this.items,
    required this.groupIndex,
    required this.itemBuilder,
  });

  final TileLayout layout;
  final List<TileSpan> spans;
  final Size size;
  final double cellSize;
  final double gapSize;
  final List<T> items;
  final int groupIndex;
  final Widget Function(BuildContext, T) itemBuilder;

  @override
  Widget build(BuildContext context) {
    return TileGrid(
      layout: layout,
      spans: spans,
      cellSize: cellSize,
      gapSize: gapSize,
      size: size,
      children: [
        for (var i = 0; i < spans.length; i++)
          ManagedTurntileScreenItem(
            groupId: groupIndex,
            row: layout.placements[i].row,
            column: layout.placements[i].column,
            columns: spans[i].columns,
            rows: spans[i].rows,
            child: itemBuilder(context, items[i]),
          ),
      ],
    );
  }
}
