import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import 'providers/start_screen_layout_manager.dart';

class ManagedStartScreenItem extends StatefulWidget {
  final int groupId;
  final int row;
  final int column;
  final double width;
  final double height;
  final Widget child;
  final String? prefix;
  final int columns;
  final int rows;

  const ManagedStartScreenItem({
    super.key,
    required this.groupId,
    required this.row,
    required this.column,
    required this.width,
    required this.height,
    required this.child,
    this.prefix,
    this.columns = 1,
    this.rows = 1,
  });

  @override
  ManagedStartScreenItemState createState() => ManagedStartScreenItemState();
}

class ManagedStartScreenItemState extends State<ManagedStartScreenItem> {
  StartScreenLayoutManager? provider;

  bool _show = false;
  StartGroupItemData? _data;
  bool _showInstantly = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();

    setState(() {
      provider = Provider.of<StartScreenLayoutManager>(context, listen: false);
      _register();
    });
  }

  @override
  void didUpdateWidget(ManagedStartScreenItem oldWidget) {
    super.didUpdateWidget(oldWidget);

    // A retained tile can move or change its span when its group lays out
    // again, and the manager must not keep the old place and size.
    if (oldWidget.groupId != widget.groupId ||
        oldWidget.row != widget.row ||
        oldWidget.column != widget.column ||
        oldWidget.prefix != widget.prefix ||
        oldWidget.columns != widget.columns ||
        oldWidget.rows != widget.rows) {
      _register(keepShown: true);
    }
  }

  void _register({bool keepShown = false}) {
    if (_data != null) {
      provider?.unregisterItem(_data!);
    }

    final registerResult = provider?.registerItem(
      widget.groupId,
      widget.row,
      widget.column,
      startAnimation,
      widget.prefix,
      widget.columns,
      widget.rows,
    );
    // A tile that is already visible does not hide again for its new place.
    _show = (keepShown && _show) || (registerResult?.skipAnimation ?? true);
    _data = registerResult?.data;
    if (_show) {
      _showInstantly = true;
    }
  }

  startAnimation() {
    if (!mounted) return;

    setState(() {
      _show = true;
    });
  }

  @override
  void dispose() {
    if (_data != null) {
      provider?.unregisterItem(_data!);
    }
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
