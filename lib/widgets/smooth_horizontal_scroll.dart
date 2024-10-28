import 'package:flutter/gestures.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../utils/lerp_controller.dart';

class SmoothScrollController extends ScrollController {
  final TickerProvider vsync;
  late final LerpController _lerpController;

  SmoothScrollController({
    super.initialScrollOffset,
    super.keepScrollOffset,
    super.debugLabel,
    required this.vsync,
  }) {
    _lerpController = LerpController(
      initialValue: initialScrollOffset,
      getter: () => offset,
      setter: (value) => jumpTo(value),
      vsync: vsync,
    );
  }

  void smoothScrollBy(double delta) {
    double targetOffset = (_lerpController.value + delta).clamp(
      0.0,
      position.maxScrollExtent,
    );
    _lerpController.lerp(targetOffset);
  }

  @override
  void dispose() {
    _lerpController.dispose();
    super.dispose();
  }
}

class SmoothHorizontalScroll extends StatefulWidget {
  final Widget Function(BuildContext, ScrollController) builder;
  final SmoothScrollController? controller;

  const SmoothHorizontalScroll({
    super.key,
    required this.builder,
    this.controller,
  });

  @override
  SmoothHorizontalScrollState createState() => SmoothHorizontalScrollState();
}

class SmoothHorizontalScrollState extends State<SmoothHorizontalScroll>
    with SingleTickerProviderStateMixin {
  late final SmoothScrollController _scrollController;
  bool _isInternalController = false;

  @override
  void initState() {
    super.initState();
    _scrollController =
        widget.controller ?? SmoothScrollController(vsync: this);
    _isInternalController = widget.controller == null;
  }

  @override
  void dispose() {
    if (_isInternalController) {
      _scrollController.dispose();
    }
    super.dispose();
  }

  void _handlePointerSignal(PointerSignalEvent signal) {
    if (signal is PointerScrollEvent) {
      double scrollDelta =
          signal.scrollDelta.dx.abs() > signal.scrollDelta.dy.abs()
              ? signal.scrollDelta.dx
              : signal.scrollDelta.dy;

      _scrollController.smoothScrollBy(scrollDelta);
    }
  }

  @override
  Widget build(BuildContext context) {
    return Listener(
      onPointerSignal: _handlePointerSignal,
      child: widget.builder(context, _scrollController),
    );
  }
}
