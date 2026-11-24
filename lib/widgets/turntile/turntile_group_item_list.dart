import 'package:fluent_ui/fluent_ui.dart';

import '../start_screen/start_group_item.dart';

import 'managed_turntile_screen_item.dart';

class TurntileGroupItemList<T> extends StatelessWidget {
  const TurntileGroupItemList({
    super.key,
    required this.finalHeight,
    required this.gapSize,
    required this.dimensions,
    required this.items,
    required this.groupIndex,
    required this.cellSize,
    required this.itemBuilder,
  });

  final double finalHeight;
  final double gapSize;
  final Dimensions dimensions;
  final List items;
  final int groupIndex;
  final double cellSize;
  final Widget Function(BuildContext, T) itemBuilder;

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      width: double.infinity,
      height: finalHeight,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: List.generate(
          dimensions.count,
          (index) {
            final int row = index ~/ dimensions.columns;
            final int column = index % dimensions.columns;
            final T item = items[index];
            return Padding(
              padding: EdgeInsets.only(bottom: gapSize),
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
