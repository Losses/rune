import 'dart:math';

import 'package:fluent_ui/fluent_ui.dart';

import '../../widgets/start_screen/start_group_item.dart';

import 'turntile_group_item_tile.dart';

class TurntileGroupItemsTile<T> extends StatelessWidget {
  final double cellSize;
  final double gapSize;
  final List<T> items;
  final int groupIndex;
  final Widget Function(BuildContext, T) itemBuilder;

  final Dimensions Function(double, double, double, List<T>)
      dimensionCalculator;

  const TurntileGroupItemsTile({
    super.key,
    required this.cellSize,
    required this.gapSize,
    required this.items,
    required this.groupIndex,
    required this.itemBuilder,
    Dimensions Function(double, double, double, List<T>)? dimensionCalculator,
  }) : dimensionCalculator = dimensionCalculator ?? _defaultDimensionCalculator;

  static Dimensions _defaultDimensionCalculator(
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

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        final Dimensions dimensions = dimensionCalculator(
          constraints.maxWidth,
          cellSize,
          gapSize,
          items,
        );

        final finalCellSize =
            (constraints.maxWidth - (dimensions.columns * gapSize)) /
                dimensions.columns;

        final double finalHeight =
            finalCellSize * dimensions.rows + gapSize * (dimensions.rows - 1);
        final double finalWidth = constraints.maxWidth;

        return TurntileGroupItemTile<T>(
          columns: dimensions.columns,
          cellSize: finalCellSize,
          finalWidth: finalWidth.floor().toDouble(),
          finalHeight: finalHeight.ceil().toDouble(),
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
