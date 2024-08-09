import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/scheduler.dart';

class SmoothHorizontalScroll extends StatefulWidget {
  final Widget Function(BuildContext, ScrollController) builder;

  const SmoothHorizontalScroll({super.key, required this.builder});

  @override
  SmoothHorizontalScrollState createState() => SmoothHorizontalScrollState();
}

class SmoothHorizontalScrollState extends State<SmoothHorizontalScroll>
    with SingleTickerProviderStateMixin {
  final ScrollController _scrollController = ScrollController();
  double _targetOffset = 0.0;
  Ticker? _ticker;

  @override
  void initState() {
    super.initState();
    _ticker = createTicker(_onTick);
  }

  @override
  void dispose() {
    _scrollController.dispose();
    _ticker?.dispose();
    super.dispose();
  }

  void _startSmoothScroll(double delta) {
    _targetOffset = (_targetOffset + delta).clamp(
      0.0,
      _scrollController.position.maxScrollExtent,
    );

    if (_ticker?.isTicking == false) {
      _ticker?.start();
    }
  }

  void _onTick(Duration elapsed) {
    if ((_scrollController.offset - _targetOffset).abs() < 1e-1) {
      _ticker?.stop();
    } else {
      double newOffset = lerpDouble(
        _scrollController.offset,
        _targetOffset,
        0.1,
      )!;
      _scrollController.jumpTo(newOffset);
    }
  }

  @override
  Widget build(BuildContext context) {
    return Listener(
      onPointerSignal: (pointerSignal) {
        if (pointerSignal is PointerScrollEvent) {
          _startSmoothScroll(pointerSignal.scrollDelta.dy);
        }
      },
      child: widget.builder(context, _scrollController),
    );
  }
}

double? lerpDouble(double a, double b, double t) {
  return a + (b - a) * t;
}
