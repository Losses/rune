import 'dart:math' as math;

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../start_screen/providers/start_screen_layout_manager.dart';

class ManagedTurntileScreenItem extends StatefulWidget {
  final int groupId;
  final int row;
  final int column;
  final Widget child;
  final String? prefix;

  const ManagedTurntileScreenItem({
    super.key,
    required this.groupId,
    required this.row,
    required this.column,
    required this.child,
    this.prefix,
  });

  @override
  ManagedTurntileScreenItemState createState() =>
      ManagedTurntileScreenItemState();
}

class ManagedTurntileScreenItemState extends State<ManagedTurntileScreenItem>
    with SingleTickerProviderStateMixin {
  StartScreenLayoutManager? provider;

  bool _show = false;
  StartGroupItemData? _data;
  bool _showInstantly = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();

    provider = Provider.of<StartScreenLayoutManager>(context, listen: false);

    if (_data != null) {
      provider?.unregisterItem(_data!);
    }

    final registerResult = provider?.registerItem(
      widget.groupId,
      widget.row,
      widget.column,
      startAnimation,
      widget.prefix,
    );
    bool newShow = registerResult?.skipAnimation ?? true;
    _data = registerResult?.data;

    if (newShow != _show) {
      setState(() {
        _show = newShow;
        _showInstantly = _show;
      });
    }
  }

  void startAnimation() {
    if (!mounted) return;

    setState(() {
      _show = true;
      _showInstantly = false;
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
    return AnimatedOpacity(
      opacity: _show ? 1.0 : 0.0,
      duration: Duration(milliseconds: _showInstantly ? 0 : 800),
      curve: Curves.easeOutQuint,
      child: TweenAnimationBuilder<double>(
        tween:
            Tween<double>(begin: _show ? 90.0 : 0.0, end: _show ? 0.0 : 90.0),
        duration: Duration(milliseconds: _showInstantly ? 0 : 800),
        curve: Curves.easeOutQuint,
        builder: (context, value, child) {
          return Transform(
            alignment: Alignment((widget.column * -1.0) - 1.4, 0.0),
            transform: Matrix4.identity()
              ..setEntry(3, 2, 0.001)
              ..rotateY(value * math.pi / 180),
            child: child,
          );
        },
        child: widget.child,
      ),
    );
  }
}
