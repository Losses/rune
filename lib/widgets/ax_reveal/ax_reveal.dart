import 'package:fluent_ui/fluent_ui.dart';

import '../collection_item.dart';
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
      _mouseDownPosition = event.localPosition;
    });

    _controller.mouseDown();
  }

  void _handleMouseUp(PointerEvent event) {
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

class AxReveal0 extends StatelessWidget {
  final Widget child;
  const AxReveal0({super.key, required this.child});

  @override
  Widget build(BuildContext context) {
    final brightness = FluentTheme.of(context).brightness;

    return AxReveal(
      config: brightness == Brightness.dark
          ? defaultLightRevealConfig
          : defaultDarkRevealConfig,
      child: child,
    );
  }
}
