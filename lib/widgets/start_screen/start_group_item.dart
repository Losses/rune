import 'package:fluent_ui/fluent_ui.dart';
import 'package:player/widgets/start_screen/providers/start_screen_layout_manager.dart';
import 'package:provider/provider.dart';

class StartGroupItem<T> extends StatefulWidget {
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
  StartGroupItemState<T> createState() => StartGroupItemState<T>();
}

class StartGroupItemState<T> extends State<StartGroupItem<T>> {
  StartScreenLayoutManager? provider;

  bool _show = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();

    setState(() {
      provider = Provider.of<StartScreenLayoutManager>(context, listen: false);
      _show = provider?.registerItem(
              widget.groupId, widget.row, widget.column, startAnimation) ??
          true;
    });
  }

  startAnimation() {
    setState(() {
      _show = true;
    });
  }

  @override
  void dispose() {
    provider?.unregisterItem(widget.groupId, widget.row, widget.column);
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      width: widget.cellSize,
      height: widget.cellSize,
      child: !_show ? Container() : widget.itemBuilder(context, widget.item),
    );
  }
}
