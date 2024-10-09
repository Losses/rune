import 'package:fluent_ui/fluent_ui.dart';

import '../start_screen/start_group_item.dart';

import 'managed_turntile_screen_item.dart';

class TurntileGroupItemTile<T> extends StatelessWidget {
  const TurntileGroupItemTile({
    super.key,
    required this.finalWidth,
    required this.finalHeight,
    required this.gapSize,
    required this.dimensions,
    required this.items,
    required this.groupIndex,
    required this.columns,
    required this.cellSize,
    required this.itemBuilder,
  });

  final double finalWidth;
  final double finalHeight;
  final double gapSize;
  final Dimensions dimensions;
  final List<T> items;
  final int groupIndex;
  final int columns;
  final double cellSize;
  final Widget Function(BuildContext, T) itemBuilder;

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      width: finalWidth,
      height: finalHeight,
      child: Wrap(
        spacing: gapSize,
        runSpacing: gapSize,
        children: List.generate(
          items.length,
          (index) {
            final int row = index ~/ dimensions.columns;
            final int column = index % dimensions.columns;
            final T item = items[index];
            return SizedBox(
              width: cellSize,
              height: cellSize,
              child: ManagedTurntileScreenItem(
                groupId: groupIndex,
                row: row,
                column: column,
                child: itemBuilder(context, item),
              ),
            );
          },
        ),
      ),
    );
  }
}
