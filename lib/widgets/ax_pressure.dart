import 'dart:math';

import 'package:flutter/material.dart';
import 'package:vector_math/vector_math_64.dart';

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
  Size? _viewportSize;
  double? _pixelRatio;
  bool _pointerActive = false;
  bool _ignorePointer = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final size = MediaQuery.sizeOf(context);
    final ratio = MediaQuery.devicePixelRatioOf(context);
    if (_viewportSize != null &&
        (size != _viewportSize || ratio != _pixelRatio)) {
      _ignorePointer = _pointerActive;
      _resetTransform();
    }
    _viewportSize = size;
    _pixelRatio = ratio;
  }

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

    final z =
        (pow(centerX.abs(), 2) + pow(centerY.abs(), 2)) /
            (pow(size.width / 2, 2) + pow(size.height / 2, 2)) -
        1;

    setState(() {
      _transform = Matrix4.identity()
        ..setEntry(3, 2, 1 / widget.perspective)
        ..rotateX(-ay * pi / 180)
        ..rotateY(-ax * pi / 180)
        ..translateByVector3(Vector3(0.0, 0.0, -z * widget.zoomFactor));
    });
  }

  @override
  Widget build(BuildContext context) {
    // Use listener to pop event to parents widget
    return Listener(
      onPointerDown: (event) {
        _pointerActive = true;
        _ignorePointer = false;
        _updateTransform(event.localPosition, context.size!);
      },
      onPointerMove: (event) {
        if (_ignorePointer) return;
        _updateTransform(event.localPosition, context.size!);
      },
      onPointerUp: (event) {
        _pointerActive = false;
        _ignorePointer = false;
        setState(() => _resetTransform());
      },
      onPointerCancel: (event) {
        _pointerActive = false;
        _ignorePointer = false;
        setState(() => _resetTransform());
      },
      onPointerPanZoomStart: (event) {
        _pointerActive = true;
        _ignorePointer = false;
        _updateTransform(event.localPosition, context.size!);
      },
      onPointerPanZoomUpdate: (event) {
        if (_ignorePointer) return;
        final localPosition = event.localPosition + event.localPan;
        _updateTransform(localPosition, context.size!);
      },
      onPointerPanZoomEnd: (event) {
        _pointerActive = false;
        _ignorePointer = false;
        setState(() => _resetTransform());
      },
      child: Container(
        transform: _transform,
        transformAlignment: Alignment.center,
        child: widget.child,
      ),
    );
  }
}
