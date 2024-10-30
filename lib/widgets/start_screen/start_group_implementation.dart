import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:fluent_ui/fluent_ui.dart';

import 'start_group_item.dart';

class StartGroupImplementation<T> extends StatelessWidget {
  final double cellSize;
  final double gapSize;
  final List<T> items;
  final int groupIndex;
  final BoxConstraints constraints;
  final Widget Function(BuildContext, T) itemBuilder;

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

  static Dimensions squareDimensionCalculator(
    double containerHeight,
    double cellSize,
    double gapSize,
    List items,
  ) {
    final int rows = max(
      ((containerHeight - 32) / (cellSize + gapSize)).floor(),
      1,
    );
    final int maxItems =
        min(pow(sqrt(items.length).floor(), 2).floor(), pow(rows, 2).floor());
    final int columns = min((maxItems / rows).ceil(), rows);
    return Dimensions(rows: rows, columns: columns, count: maxItems);
  }

  static (double, double) finalSizeCalculator(
    Dimensions dimensions,
    double cellSize,
    double gapSize,
  ) {
    final double finalHeight = clampDouble(
      dimensions.rows * (cellSize + gapSize) - gapSize,
      0,
      double.infinity,
    );

    final double finalWidth = clampDouble(
      dimensions.columns * (cellSize + gapSize) - gapSize,
      0,
      double.infinity,
    );

    return (finalWidth, finalHeight);
  }

  @override
  Widget build(BuildContext context) {
    final Dimensions dimensions = dimensionCalculator(
      constraints.maxHeight,
      cellSize,
      gapSize,
      items,
    );

    final (finalWidth, finalHeight) = finalSizeCalculator(
      dimensions,
      cellSize,
      gapSize,
    );

    return StartGroupItem<T>(
      finalWidth: finalWidth,
      finalHeight: finalHeight,
      gapSize: gapSize,
      dimensions: dimensions,
      items: items,
      groupIndex: groupIndex,
      cellSize: cellSize,
      itemBuilder: itemBuilder,
    );
  }
}
