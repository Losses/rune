import 'dart:math';

import 'package:fluent_ui/fluent_ui.dart';

import '../tile_group/tile_grid.dart';
import '../tile_group/tile_layout.dart';
import 'start_group_item.dart';

class StartGroupImplementation<T> extends StatelessWidget {
  final double cellSize;
  final double gapSize;
  final List<T> items;
  final int groupIndex;
  final BoxConstraints constraints;
  final Widget Function(BuildContext, T) itemBuilder;
  final Axis direction;
  final TileSpan Function(T item)? tileSpanBuilder;
  final int? strip;

  final Dimensions Function(double, double, double, List<T>)
  dimensionCalculator;

  const StartGroupImplementation({
    super.key,
    required this.cellSize,
    required this.gapSize,
    required this.items,
    required this.groupIndex,
    required this.constraints,
    required this.itemBuilder,
    required this.direction,
    this.tileSpanBuilder,
    this.strip,
    Dimensions Function(double, double, double, List<T>)? dimensionCalculator,
  }) : dimensionCalculator = dimensionCalculator ?? defaultDimensionCalculator;

  const StartGroupImplementation.square({
    super.key,
    required this.cellSize,
    required this.gapSize,
    required this.items,
    required this.groupIndex,
    required this.constraints,
    required this.itemBuilder,
    required this.direction,
    this.tileSpanBuilder,
    this.strip,
  }) : dimensionCalculator = squareDimensionCalculator;

  static Dimensions defaultDimensionCalculator(
    double containerHeight,
    double cellSize,
    double gapSize,
    List items,
  ) {
    final int rows = max(
      ((containerHeight - 24) / (cellSize + gapSize)).floor(),
      1,
    );
    final int columns = (items.length / rows).ceil();
    return Dimensions(rows: rows, columns: columns, count: items.length);
  }

  static Dimensions startLinkDimensionCalculator(
    double containerHeight,
    double cellSize,
    double gapSize,
    List items,
  ) {
    final int rows = max(
      ((containerHeight - 32) / (cellSize + gapSize)).floor(),
      1,
    );
    final int columns = (items.length / rows).ceil();
    return Dimensions(rows: rows, columns: columns, count: items.length);
  }

  static Dimensions squareDimensionCalculator(
    double containerHeight,
    double cellSize,
    double gapSize,
    List items,
  ) {
    final int columns = max(
      ((containerHeight - 32) / (cellSize + gapSize)).floor(),
      1,
    );
    final int maxItems = min(
      pow(sqrt(items.length).floor(), 2).floor(),
      pow(columns, 2).floor(),
    );
    final int rows = min((maxItems / columns).ceil(), columns);

    final int trueColumns = max(columns, 3);
    final int correctedMaxItems = min(trueColumns * rows, items.length);

    final notEnough = items.length < trueColumns * rows;

    return Dimensions(
      rows: notEnough ? trueColumns : rows,
      columns: notEnough ? rows : trueColumns,
      count: correctedMaxItems,
    );
  }

  /// Places the first `d.count` tiles the way the dimension calculators
  /// count them: `d.rows` rows are fixed and the group grows to the right.
  /// For [Axis.horizontal] tiles fill row by row, in strips as wide as
  /// `d.columns`. For [Axis.vertical] tiles fill column by column. A [strip]
  /// given overrides the strip width that [direction] implies.
  static TileLayout tileLayout(
    Dimensions d,
    Axis direction,
    List<TileSpan> spans, {
    int? strip,
  }) {
    return layoutTiles(
      spans,
      rows: d.rows,
      strip: strip ?? (direction == Axis.horizontal ? max(1, d.columns) : 1),
    );
  }

  // A tile never sticks out of its group, where it couldn't be tapped.
  static TileSpan _capRows(TileSpan span, int rows) =>
      TileSpan(span.columns, min(span.rows, rows), newColumn: span.newColumn);

  /// The layout of the group, the spans of the tiles shown and the box of the
  /// group: as wide as the columns in use, as high as the rows of the group.
  ({TileLayout layout, List<TileSpan> spans, Size size}) measure() {
    Dimensions d = dimensionCalculator(
      constraints.maxHeight,
      cellSize,
      gapSize,
      items,
    );
    final groupRows = max(1, d.rows);
    var spans = [
      for (var i = 0; i < d.count; i++)
        _capRows(
          tileSpanBuilder?.call(items[i]) ?? const TileSpan(1, 1),
          groupRows,
        ),
    ];
    if (spans.any((s) => s.columns != 1 || s.rows != 1)) {
      // The calculators count items as cells. With larger tiles they get a
      // list as long as the cells of all tiles, which gives the block in
      // cells. If that capped the count, the tiles shown are the longest
      // prefix whose layout stays within the block's columns.
      final all = [
        for (final item in items)
          _capRows(
            tileSpanBuilder?.call(item) ?? const TileSpan(1, 1),
            groupRows,
          ),
      ];
      final cells = all.fold<int>(0, (sum, s) => sum + s.columns * s.rows);
      d = dimensionCalculator(
        constraints.maxHeight,
        cellSize,
        gapSize,
        List<T>.filled(cells, items.first),
      );
      final capped = max(1, d.rows);
      spans = [for (final s in all) _capRows(s, capped)];
      if (d.count < cells) {
        var shown = spans.length;
        while (shown > 1 &&
            tileLayout(
                  d,
                  direction,
                  spans.sublist(0, shown),
                  strip: strip,
                ).columns >
                d.columns) {
          shown--;
        }
        spans = spans.sublist(0, shown);
      }
    }
    final layout = tileLayout(d, direction, spans, strip: strip);
    final size = Size(
      TileGrid.extent(max(layout.columns, d.columns), cellSize, gapSize),
      TileGrid.extent(d.rows, cellSize, gapSize),
    );
    return (layout: layout, spans: spans, size: size);
  }

  @override
  Widget build(BuildContext context) {
    final m = measure();

    return StartGroupItem<T>(
      layout: m.layout,
      spans: m.spans,
      size: m.size,
      gapSize: gapSize,
      items: items,
      groupIndex: groupIndex,
      cellSize: cellSize,
      itemBuilder: itemBuilder,
    );
  }
}
