import 'dart:math';

import 'package:fluent_ui/fluent_ui.dart';

import './start_group_item.dart';

class Dimensions<T> {
  final int rows;
  final int columns;
  final int count;

  Dimensions({
    required this.rows,
    required this.columns,
    required this.count,
  });

  @override
  String toString() => "Dimensions($rows x $columns, count: $count)";
}

class StartGroupItems<T> extends StatelessWidget {
  final double cellSize;
  final double gapSize;
  final List<T> items;
  final int groupIndex;
  final Widget Function(BuildContext, T) itemBuilder;

  final Dimensions Function(double, double, double, List<T>)
      dimensionCalculator;

  const StartGroupItems({
    super.key,
    required this.cellSize,
    required this.gapSize,
    required this.items,
    required this.groupIndex,
    required this.itemBuilder,
    Dimensions Function(double, double, double, List<T>)? dimensionCalculator,
  }) : dimensionCalculator = dimensionCalculator ?? _defaultDimensionCalculator;

  const StartGroupItems.square({
    super.key,
    required this.cellSize,
    required this.gapSize,
    required this.items,
    required this.groupIndex,
    required this.itemBuilder,
  }) : dimensionCalculator = _squareDimensionCalculator;

  static Dimensions _defaultDimensionCalculator(
      double containerHeight, double cellSize, double gapSize, List items) {
    final int rows = (containerHeight / (cellSize + gapSize)).floor();
    final int columns = (items.length / rows).ceil();
    return Dimensions(rows: rows, columns: columns, count: items.length);
  }

  static Dimensions _squareDimensionCalculator(
      double containerHeight, double cellSize, double gapSize, List items) {
    final int rows = (containerHeight / (cellSize + gapSize)).floor();
    final int maxItems =
        min(pow(sqrt(items.length).floor(), 2).floor(), pow(rows, 2).floor());
    final int columns = min((maxItems / rows).ceil(), rows);
    return Dimensions(rows: rows, columns: columns, count: maxItems);
  }

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        final Dimensions dimensions = dimensionCalculator(
            constraints.maxHeight, cellSize, gapSize, items);

        final double finalHeight =
            dimensions.rows * (cellSize + gapSize) - gapSize;
        final double finalWidth =
            dimensions.columns * (cellSize + gapSize) - gapSize;

        return SizedBox(
          width: finalWidth,
          height: finalHeight,
          child: Wrap(
            spacing: gapSize,
            runSpacing: gapSize,
            children: List.generate(dimensions.count, (index) {
              final int row = index ~/ dimensions.columns;
              final int column = index % dimensions.columns;
              final T item = items[index];
              return StartGroupItem(
                cellSize: cellSize,
                itemBuilder: itemBuilder,
                item: item,
                groupId: groupIndex,
                row: row,
                column: column,
              );
            }),
          ),
        );
      },
    );
  }
}
