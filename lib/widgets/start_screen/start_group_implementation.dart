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

  // Whether the group is a square block, whose width is fixed.
  final bool _square;

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
  }) : dimensionCalculator = dimensionCalculator ?? defaultDimensionCalculator,
       _square = false;

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
  }) : dimensionCalculator = squareDimensionCalculator,
       _square = true;

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

  // A tile never sticks out of its group, where it couldn't be tapped, and
  // never covers less than a cell. A span callback is not trusted to get
  // either right.
  static TileSpan _fit(TileSpan span, int rows, [int? columns]) => TileSpan(
    max(1, min(span.columns, columns ?? span.columns)),
    max(1, min(span.rows, rows)),
    newColumn: span.newColumn,
  );

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
    final raw = [
      for (final item in items)
        tileSpanBuilder?.call(item) ?? const TileSpan(1, 1),
    ];
    var spans = [for (var i = 0; i < d.count; i++) _fit(raw[i], groupRows)];
    if (spans.any((s) => s.columns != 1 || s.rows != 1)) {
      // The calculators only count the items they get, so they are given a
      // list as long as the cells of all tiles, which gives the block in
      // cells. The cells are counted with the rows of the first block, but
      // the tiles are capped again from their own size with the rows of the
      // final one, so a tile that the first block cut keeps what the final
      // block allows. If that capped the count, the tiles shown are the
      // longest prefix whose layout stays within the block's columns.
      final cells = raw.fold<int>(0, (sum, s) {
        final fitted = _fit(s, groupRows);
        return sum + fitted.columns * fitted.rows;
      });
      d = dimensionCalculator(
        constraints.maxHeight,
        cellSize,
        gapSize,
        List<T>.filled(cells, items.first),
      );
      final capped = max(1, d.rows);
      spans = [for (final s in raw) _fit(s, capped)];
      if (_square) {
        // A square block has a fixed width: a tile wider than it would make
        // the group wider than it, even alone, and push the next group away.
        final block = max(1, d.columns);
        spans = [for (final s in spans) _fit(s, capped, block)];
      }
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
