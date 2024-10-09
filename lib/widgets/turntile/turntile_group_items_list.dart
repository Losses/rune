import 'package:flutter/foundation.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../widgets/start_screen/start_group_item.dart';

import 'turntile_group_item_list.dart';

class TurntileGroupItemsList<T> extends StatelessWidget {
  final double cellSize;
  final double gapSize;
  final List<T> items;
  final int groupIndex;
  final Widget Function(BuildContext, T) itemBuilder;

  final Dimensions Function(double, double, double, List<T>)
      dimensionCalculator;

  const TurntileGroupItemsList({
    super.key,
    required this.cellSize,
    required this.gapSize,
    required this.items,
    required this.groupIndex,
    required this.itemBuilder,
    Dimensions Function(double, double, double, List<T>)? dimensionCalculator,
  }) : dimensionCalculator = dimensionCalculator ?? _defaultDimensionCalculator;

  static Dimensions _defaultDimensionCalculator(
      double containerHeight, double cellSize, double gapSize, List items) {
    final int rows = items.length;
    return Dimensions(rows: rows, columns: 1, count: items.length);
  }

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
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

        return TurntileGroupItemList<T>(
          cellSize: cellSize,
          finalHeight: finalHeight,
          gapSize: gapSize,
          dimensions: dimensions,
          items: items,
          groupIndex: groupIndex,
          itemBuilder: itemBuilder,
        );
      },
    );
  }
}
