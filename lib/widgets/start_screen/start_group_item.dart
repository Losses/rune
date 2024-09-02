import 'package:fluent_ui/fluent_ui.dart';

class StartGroupItem<T> extends StatelessWidget {
  const StartGroupItem({
    super.key,
    required this.cellSize,
    required this.item,
    required this.itemBuilder,
    required this.groupId,
    required this.row,
    required this.column,
  });

  final double cellSize;
  final T item;
  final Widget Function(BuildContext, T) itemBuilder;
  final int groupId;
  final int row;
  final int column;

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      width: cellSize,
      height: cellSize,
      child: itemBuilder(context, item),
    );
  }
}
