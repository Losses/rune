import 'package:fluent_ui/fluent_ui.dart';

import 'utils/reveal_config.dart';
import 'utils/reveal_effect_painter.dart';

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

class _AxRevealState extends State<AxReveal>
    with SingleTickerProviderStateMixin {
  Offset? _mousePosition;
  bool _mousePressed = false;
  bool _mouseReleased = false;
  Offset? _mouseUpPosition;
  late AnimationController _pressAnimationController;

  @override
  void initState() {
    super.initState();
    _pressAnimationController = AnimationController(
      vsync: this,
      duration: const Duration(milliseconds: 400),
    );
  }

  @override
  void dispose() {
    _pressAnimationController.dispose();
    super.dispose();
  }

  void _handleMouseMove(PointerEvent event) {
    setState(() {
      _mousePosition = event.localPosition;
    });
  }

  void _handleMouseDown(PointerEvent event) {
    setState(() {
      _mousePressed = true;
      _mouseReleased = false;
      _mouseUpPosition = null;
    });
    _pressAnimationController.forward(from: 0);
  }

  void _handleMouseUp(PointerEvent event) {
    setState(() {
      _mousePressed = false;
      _mouseReleased = true;
      _mouseUpPosition = event.localPosition;
    });
  }

  void _handleMouseExit(PointerEvent event) {
    setState(() {
      _mousePosition = null;
    });
  }

  @override
  Widget build(BuildContext context) {
    return MouseRegion(
      onHover: _handleMouseMove,
      onExit: _handleMouseExit,
      child: Listener(
        onPointerDown: _handleMouseDown,
        onPointerUp: _handleMouseUp,
        onPointerMove: _handleMouseMove,
        child: CustomPaint(
          foregroundPainter: RevealEffectPainter(
            mousePosition: _mousePosition,
            mousePressed: _mousePressed,
            mouseReleased: _mouseReleased,
            mouseUpPosition: _mouseUpPosition,
            mouseDownAnimateFrame: _pressAnimationController.value,
            config: widget.config,
          ),
          child: widget.child,
        ),
      ),
    );
  }
}
