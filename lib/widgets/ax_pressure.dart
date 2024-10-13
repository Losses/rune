import 'dart:math';

import 'package:flutter/material.dart';

enum TiltMode { absolute, relateive }

class AxPressure extends StatefulWidget {
  final Widget child;
  final bool invert;
  final double tiltFactor;
  final double tiltDepth;
  final double perspective;
  final double zoomFactor;
  final TiltMode tiltMode;

  const AxPressure({
    super.key,
    required this.child,
    this.invert = false,
    this.tiltFactor = 10,
    this.tiltDepth = 50,
    this.perspective = 800,
    this.zoomFactor = 16,
    this.tiltMode = TiltMode.relateive,
  });

  @override
  AxPressureState createState() => AxPressureState();
}

class AxPressureState extends State<AxPressure> {
  Matrix4? _transform;

  @override
  void initState() {
    super.initState();
    _resetTransform();
  }

  void _resetTransform() {
    _transform = Matrix4.identity()..setEntry(3, 2, 1 / widget.perspective);
  }

  void _updateTransform(Offset localPosition, Size size) {
    final halfW = size.width / 2;
    final halfH = size.height / 2;

    final centerX = (localPosition.dx - halfW).clamp(-halfW, halfW);
    final centerY = (localPosition.dy - halfH).clamp(-halfH, halfH);

    List<double> degFactors;
    if (widget.tiltMode == TiltMode.absolute) {
      final sinX = widget.tiltDepth / halfW;
      final sinY = widget.tiltDepth / halfH;
      final degX = asin(sinX.clamp(-0.99, 0.99)) * 180 / pi;
      final degY = asin(sinY.clamp(-0.99, 0.99)) * 180 / pi;
      degFactors = [degX, degY];
    } else if (widget.tiltMode == TiltMode.relateive) {
      degFactors = [widget.tiltFactor, widget.tiltFactor];
    } else {
      throw ArgumentError('tiltMode should be "absolute" or "relative"');
    }

    final ax =
        (centerX / size.width) * degFactors[0] * (widget.invert ? -1 : 1);
    final ay =
        (centerY / size.height) * degFactors[1] * (widget.invert ? 1 : -1);

    final z = (pow(centerX.abs(), 2) + pow(centerY.abs(), 2)) /
            (pow(size.width / 2, 2) + pow(size.height / 2, 2)) -
        1;

    setState(() {
      _transform = Matrix4.identity()
        ..setEntry(3, 2, 1 / widget.perspective)
        ..rotateX(-ay * pi / 180)
        ..rotateY(-ax * pi / 180)
        ..translate(0.0, 0.0, -z * widget.zoomFactor);
    });
  }

  @override
  Widget build(BuildContext context) {
    return GestureDetector(
      onPanStart: (details) =>
          _updateTransform(details.localPosition, context.size!),
      onPanUpdate: (details) =>
          _updateTransform(details.localPosition, context.size!),
      onTapDown: (details) =>
          _updateTransform(details.localPosition, context.size!),
      onPanEnd: (_) => setState(() => _resetTransform()),
      onPanCancel: () => setState(() => _resetTransform()),
      onTapUp: (details) => setState(() => _resetTransform()),
      child: AnimatedContainer(
        duration: const Duration(milliseconds: 50),
        transform: _transform,
        transformAlignment: Alignment.center,
        child: widget.child,
      ),
    );
  }
}
