import 'package:fluent_ui/fluent_ui.dart';

import '../tile_group/tile_grid.dart';
import '../tile_group/tile_layout.dart';
import 'managed_start_screen_item.dart';

class Dimensions {
  final int rows;
  final int columns;
  final int count;

  Dimensions({
    required this.rows,
    required this.columns,
    required this.count,
  });

  @override
  int get hashCode => Object.hash(rows, columns, count);

  @override
  bool operator ==(Object other) {
    if (identical(this, other)) return true;
    return other is Dimensions &&
        other.rows == rows &&
        other.columns == columns &&
        other.count == count;
  }

  @override
  String toString() => "Dimensions(r$rows x c$columns, count: $count)";
}

class StartGroupItem<T> extends StatelessWidget {
  const StartGroupItem({
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
          _item(context, i),
      ],
    );
  }

  Widget _item(BuildContext context, int i) {
    final p = layout.placements[i];
    final rect = TileGrid.rectOf(p, spans[i], cellSize, gapSize);
    return ManagedStartScreenItem(
      groupId: groupIndex,
      row: p.row,
      column: p.column,
      columns: spans[i].columns,
      rows: spans[i].rows,
      width: rect.width,
      height: rect.height,
      child: itemBuilder(context, items[i]),
    );
  }
}
