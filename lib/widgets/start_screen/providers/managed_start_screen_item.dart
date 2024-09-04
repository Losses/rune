import 'package:fluent_ui/fluent_ui.dart';
import 'package:player/widgets/start_screen/providers/start_screen_layout_manager.dart';
import 'package:provider/provider.dart';

class ManagedStartScreenItem extends StatefulWidget {
  final int groupId;
  final int row;
  final int column;
  final double width;
  final double height;
  final Widget child;

  const ManagedStartScreenItem({
    super.key,
    required this.groupId,
    required this.row,
    required this.column,
    required this.width,
    required this.height,
    required this.child,
  });

  @override
  ManagedStartScreenItemState createState() => ManagedStartScreenItemState();
}

class ManagedStartScreenItemState extends State<ManagedStartScreenItem> {
  StartScreenLayoutManager? provider;

  bool _show = false;
  bool _showInstantly = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();

    setState(() {
      provider = Provider.of<StartScreenLayoutManager>(context, listen: false);
      _show = provider?.registerItem(
              widget.groupId, widget.row, widget.column, startAnimation) ??
          true;

      if (_show) {
        _showInstantly = true;
      }
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
      width: widget.width,
      height: widget.height,
      child: AnimatedOpacity(
        opacity: _show ? 1.0 : 0.0,
        duration: Duration(milliseconds: _showInstantly ? 0 : 300),
        child: _show ? widget.child : Container(),
      ),
    );
  }
}
