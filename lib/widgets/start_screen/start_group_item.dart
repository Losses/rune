import 'package:fluent_ui/fluent_ui.dart';

import './providers/managed_start_screen_item.dart';

class StartGroupItem<T> extends StatelessWidget {
  final double cellSize;
  final T item;
  final Widget Function(BuildContext, T) itemBuilder;
  final int groupId;
  final int row;
  final int column;

  const StartGroupItem({
    super.key,
    required this.cellSize,
    required this.item,
    required this.itemBuilder,
    required this.groupId,
    required this.row,
    required this.column,
  });

  @override
  Widget build(BuildContext context) {
    return ManagedStartScreenItem(
        groupId: groupId,
        row: row,
        column: column,
        width: cellSize,
        height: cellSize,
        child: itemBuilder(context, item));
  }
}
