import 'dart:math';

import 'package:fluent_ui/fluent_ui.dart';

import '../../widgets/start_screen/start_group_item.dart';
import '../tile_group/tile_layout.dart';

import 'turntile_group_item_tile.dart';

class TurntileGroupItemsTile<T> extends StatelessWidget {
  final double cellSize;
  final double gapSize;
  final List<T> items;
  final int groupIndex;
  final Widget Function(BuildContext, T) itemBuilder;
  final TileSpan Function(T item)? tileSpanBuilder;
  final int strip;

  final Dimensions Function(double, double, double, List<T>)
      dimensionCalculator;

  const TurntileGroupItemsTile({
    super.key,
    required this.cellSize,
    required this.gapSize,
    required this.items,
    required this.groupIndex,
    required this.itemBuilder,
    this.tileSpanBuilder,
    this.strip = 1,
    Dimensions Function(double, double, double, List<T>)? dimensionCalculator,
  }) : dimensionCalculator = dimensionCalculator ?? defaultTurntileDimensionCalculator;

  static Dimensions defaultTurntileDimensionCalculator(
    double containerWidth,
    double cellSize,
    double gapSize,
    List items,
  ) {
    final int columns = max(
      ((containerWidth / (cellSize + gapSize)).floor()),
      1,
    );
    final int rows = (items.length / columns).ceil();

    return Dimensions(rows: rows, columns: columns, count: items.length);
  }

  static TileSpan _capColumns(TileSpan span, int columns) => TileSpan(
        min(span.columns, max(1, columns)),
        span.rows,
        newColumn: span.newColumn,
      );

  /// The layout of the tiles in a box [width] wide, the size of one cell and
  /// the box of the group.
  ({TileLayout layout, List<TileSpan> spans, double cellSize, Size size})
      measure(double width) {
    final Dimensions dimensions = dimensionCalculator(
      width,
      cellSize,
      gapSize,
      items,
    );
    // A tile never sticks out of its group, where it couldn't be tapped.
    final spans = [
      for (final item in items)
        _capColumns(
          tileSpanBuilder?.call(item) ?? const TileSpan(1, 1),
          dimensions.columns,
        ),
    ];
    final layout = layoutTiles(
      spans,
      orientation: Axis.vertical,
      columns: dimensions.columns,
      strip: strip,
    );
    final finalCellSize =
        (width - (dimensions.columns * gapSize)) / dimensions.columns;
    final size = Size(
      width.floor().toDouble(),
      max(
        0,
        (finalCellSize * layout.rows + gapSize * (layout.rows - 1)).ceil(),
      ).toDouble(),
    );
    return (
      layout: layout,
      spans: spans,
      cellSize: finalCellSize,
      size: size,
    );
  }

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        final m = measure(constraints.maxWidth);

        return TurntileGroupItemTile<T>(
          layout: m.layout,
          spans: m.spans,
          size: m.size,
          cellSize: m.cellSize,
          gapSize: gapSize,
          items: items,
          groupIndex: groupIndex,
          itemBuilder: itemBuilder,
        );
      },
    );
  }
}
