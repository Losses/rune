import 'package:flutter/gestures.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../utils/lerp_controller.dart';

class SmoothHorizontalScroll extends StatefulWidget {
  final Widget Function(BuildContext, ScrollController) builder;

  const SmoothHorizontalScroll({super.key, required this.builder});

  @override
  SmoothHorizontalScrollState createState() => SmoothHorizontalScrollState();
}

class SmoothHorizontalScrollState extends State<SmoothHorizontalScroll>
    with SingleTickerProviderStateMixin {
  final ScrollController _scrollController = ScrollController();
  late LerpController _lerpController;

  @override
  void initState() {
    super.initState();
    _lerpController = LerpController(
      initialValue: 0.0,
      getter: () => _scrollController.offset.toDouble(),
      setter: (value) => _scrollController.jumpTo(value),
      vsync: this,
    );
  }

  @override
  void dispose() {
    _scrollController.dispose();
    _lerpController.dispose();
    super.dispose();
  }

  void _startSmoothScroll(double delta) {
    double targetOffset = (_lerpController.value + delta).clamp(
      0.0,
      _scrollController.position.maxScrollExtent,
    );

    _lerpController.lerp(targetOffset);
  }

  @override
  Widget build(BuildContext context) {
    return Listener(
      onPointerSignal: (pointerSignal) {
        if (pointerSignal is PointerScrollEvent) {
          double scrollDelta = pointerSignal.scrollDelta.dx.abs() >
                  pointerSignal.scrollDelta.dy.abs()
              ? pointerSignal.scrollDelta.dx
              : pointerSignal.scrollDelta.dy;

          _startSmoothScroll(scrollDelta);
        }
      },
      child: widget.builder(context, _scrollController),
    );
  }
}
