import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:fluent_ui/fluent_ui.dart';

import 'start_group_item.dart';

class StartGroupItems<T> extends StatelessWidget {
  final double cellSize;
  final double gapSize;
  final List<T> items;
  final int groupIndex;
  final BoxConstraints constraints;
  final Widget Function(BuildContext, T) itemBuilder;

  final Dimensions Function(double, double, double, List<T>)
      dimensionCalculator;

  const StartGroupItems({
    super.key,
    required this.cellSize,
    required this.gapSize,
    required this.items,
    required this.groupIndex,
    required this.constraints,
    required this.itemBuilder,
    Dimensions Function(double, double, double, List<T>)? dimensionCalculator,
  }) : dimensionCalculator = dimensionCalculator ?? _defaultDimensionCalculator;

  const StartGroupItems.square({
    super.key,
    required this.cellSize,
    required this.gapSize,
    required this.items,
    required this.groupIndex,
    required this.constraints,
    required this.itemBuilder,
  }) : dimensionCalculator = _squareDimensionCalculator;

  static Dimensions _defaultDimensionCalculator(
      double containerHeight, double cellSize, double gapSize, List items) {
    final int rows = ((containerHeight - 24) / (cellSize + gapSize))
        .floor()
        .clamp(1, 0x7FFFFFFFFFFFFFFF);
    final int columns = (items.length / rows).ceil();
    return Dimensions(rows: rows, columns: columns, count: items.length);
  }

  static Dimensions _squareDimensionCalculator(
      double containerHeight, double cellSize, double gapSize, List items) {
    final int rows = (containerHeight / (cellSize + gapSize))
        .floor()
        .clamp(1, 0x7FFFFFFFFFFFFFFF);
    final int maxItems =
        min(pow(sqrt(items.length).floor(), 2).floor(), pow(rows, 2).floor());
    final int columns = min((maxItems / rows).ceil(), rows);
    return Dimensions(rows: rows, columns: columns, count: maxItems);
  }

  @override
  Widget build(BuildContext context) {
    final Dimensions dimensions = dimensionCalculator(
      constraints.maxHeight,
      cellSize,
      gapSize,
      items,
    );

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
