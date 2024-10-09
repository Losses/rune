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
  late AnimationController _controller;
  late Animation<double> _animation;

  @override
  void initState() {
    super.initState();
    _controller = AnimationController(
      duration: const Duration(milliseconds: 300),
      vsync: this,
    );
    _animation = Tween<double>(begin: 90.0, end: 0.0).animate(_controller);
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();

    setState(() {
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
      _show = registerResult?.skipAnimation ?? true;
      _data = registerResult?.data;

      if (_show) {
        _showInstantly = true;
        _controller.forward();
      }
    });
  }

  void startAnimation() {
    if (!mounted) return;

    setState(() {
      _show = true;
      _controller.forward();
    });
  }

  @override
  void dispose() {
    if (_data != null) {
      provider?.unregisterItem(_data!);
    }
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return AnimatedOpacity(
      opacity: _show ? 1.0 : 0.0,
      duration: Duration(milliseconds: _showInstantly ? 0 : 300),
      child: _show
          ? AnimatedBuilder(
              animation: _animation,
              builder: (context, child) {
                return Transform(
                  alignment: const Alignment(-1.4, 0.0),
                  transform: Matrix4.identity()
                    ..setEntry(3, 2, 0.001)
                    ..rotateY(_animation.value * math.pi / 180),
                  child: child,
                );
              },
              child: widget.child,
            )
          : Container(),
    );
  }
}
