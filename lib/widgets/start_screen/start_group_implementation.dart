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
  final Axis direction;

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
      direction: direction,
    );
  }
}
