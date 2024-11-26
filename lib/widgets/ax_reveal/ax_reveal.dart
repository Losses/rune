import 'package:fluent_ui/fluent_ui.dart';

import 'utils/reveal_config.dart';
import 'utils/reveal_effect_painter.dart';
import 'utils/reveal_effect_controller.dart';

class AxReveal extends StatefulWidget {
  final Widget child;
  final RevealConfig config;

  const AxReveal({
    super.key,
    required this.child,
    this.config = const RevealConfig(),
  });

  @override
  State<AxReveal> createState() => _AxRevealState();
}

class _AxRevealState extends State<AxReveal> {
  late final RevealEffectController _controller;
  bool _mousePressed = false;
  Offset? _mouseDownPosition;

  @override
  didChangeDependencies() {
    super.didChangeDependencies();
    _controller = RevealEffectController(context, widget.config);
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  void _handleMouseDown(PointerEvent event) {
    setState(() {
      _mousePressed = true;
      _mouseDownPosition = event.localPosition;
    });

    _controller.mouseDown();
  }

  void _handleMouseUp(PointerEvent event) {
    setState(() {
      _mousePressed = false;
    });

    _controller.mouseUp();
  }

  @override
  Widget build(BuildContext context) {
    return Listener(
      key: _controller.widgetKey,
      onPointerDown: _handleMouseDown,
      onPointerUp: _handleMouseUp,
      child: ListenableBuilder(
        listenable: _controller,
        builder: (context, child) {
          return CustomPaint(
            foregroundPainter: RevealEffectPainter(
              mousePosition: _controller.localPosition,
              mousePressed: _mousePressed,
              mouseReleased: _controller.mouseReleased,
              mouseDownPosition: _mouseDownPosition,
              logicFrame: _controller.mouseDownAnimateLogicFrame,
              config: widget.config,
            ),
            child: child,
          );
        },
        child: widget.child,
      ),
    );
  }
}
