import 'package:fluent_ui/fluent_ui.dart';

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
  String toString() => "Dimensions($rows x $columns, count: $count)";
}

class StartGroupItem<T> extends StatelessWidget {
  const StartGroupItem({
    super.key,
    required this.finalWidth,
    required this.finalHeight,
    required this.gapSize,
    required this.dimensions,
    required this.items,
    required this.groupIndex,
    required this.cellSize,
    required this.itemBuilder,
    required this.direction,
  });

  final double finalWidth;
  final double finalHeight;
  final double gapSize;
  final Dimensions dimensions;
  final List items;
  final int groupIndex;
  final double cellSize;
  final Widget Function(BuildContext, T) itemBuilder;
  final Axis direction;

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      width: finalWidth,
      height: finalHeight,
      child: Wrap(
        spacing: gapSize,
        runSpacing: gapSize,
        direction: direction,
        children: List.generate(dimensions.count, (index) {
          final int row = index ~/ dimensions.columns;
          final int column = index % dimensions.columns;
          final T item = items[index];
          return ManagedStartScreenItem(
            groupId: groupIndex,
            row: row,
            column: column,
            width: cellSize,
            height: cellSize,
            child: itemBuilder(context, item),
          );
        }),
      ),
    );
  }
}
