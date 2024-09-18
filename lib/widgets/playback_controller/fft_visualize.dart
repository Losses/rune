import 'dart:ui' as ui;
import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/scheduler.dart';

import '../../messages/playback.pb.dart';

class FFTVisualize extends StatefulWidget {
  const FFTVisualize({super.key});

  @override
  FFTVisualizeState createState() => FFTVisualizeState();
}

class FFTVisualizeState extends State<FFTVisualize>
    with TickerProviderStateMixin {
  final radius = 12.0;
  List<double> _currentFftValues = [];
  List<double> _targetFftValues = [];
  Ticker? _ticker;
  bool _hasData = false;
  int _lastUpdateTime = 0;

  @override
  void initState() {
    super.initState();
    RealtimeFFT.rustSignalStream.listen((rustSignal) {
      if (!mounted) return;

      setState(() {
        _targetFftValues = rustSignal.message.value;
        _lastUpdateTime = DateTime.now().millisecondsSinceEpoch;
        if (!_hasData) {
          _hasData = true;
          _ticker?.start();
        }
      });
    });

    _ticker = createTicker((Duration elapsed) {
      final now = DateTime.now().millisecondsSinceEpoch;
      if (now - _lastUpdateTime > 42) {
        if (mounted) {
          final reduced = _currentFftValues.reduce((a, b) => a + b);
          if (!_hasData && reduced < 1e-2) {
            _currentFftValues = List.filled(_currentFftValues.length, 0.0);
            _ticker?.stop();
          }

          setState(() {
            _targetFftValues = List.filled(_currentFftValues.length, 0.0);
            _hasData = false;
          });
        }
      }

      setState(() {
        _lerpedFftValues();
      });
    });
  }

  @override
  Widget build(BuildContext context) {
    final color = FluentTheme.of(context).accentColor;

    return LayoutBuilder(builder: (context, constraints) {
      double parentHeight = constraints.maxHeight;

      return OverflowBox(
          maxHeight: parentHeight * 2,
          alignment: Alignment.topCenter,
          child: SizedBox(
            height: parentHeight * 2,
            child: Opacity(
                opacity: 0.87,
                child: ImageFiltered(
                  imageFilter:
                      ui.ImageFilter.blur(sigmaX: radius, sigmaY: radius),
                  child: CustomPaint(
                    painter: FFTPainter(_currentFftValues, color),
                  ),
                )),
          ));
    });
  }

  void _lerpedFftValues() {
    if (_targetFftValues.isEmpty) {
      return;
    }

    if (_currentFftValues.isEmpty) {
      _currentFftValues = List.filled(_targetFftValues.length, 0.0);
    }

    for (int i = 0; i < _currentFftValues.length; i++) {
      double current = _currentFftValues[i];
      double target = _targetFftValues[i];

      if (!current.isFinite) {
        current = 0.0;
      }
      if (!target.isFinite) {
        target = 0.0;
      }

      _currentFftValues[i] = ui.lerpDouble(current, target, 0.2)!;
    }
  }

  @override
  void dispose() {
    _ticker?.dispose();
    super.dispose();
  }
}

class FFTPainter extends CustomPainter {
  final List<double> fftValues;
  final Color color;

  FFTPainter(this.fftValues, this.color);

  @override
  void paint(Canvas canvas, Size size) {
    final paint = Paint()
      ..color = color
      ..strokeWidth = size.width / fftValues.length;

    final midY = size.height / 2;

    for (int i = 0; i < fftValues.length; i++) {
      final x = i * (size.width / fftValues.length);
      final y = fftValues[i] * size.height / 2;
      canvas.drawLine(Offset(x, midY - y), Offset(x, midY + y), paint);
    }
  }

  @override
  bool shouldRepaint(covariant CustomPainter oldDelegate) {
    return true;
  }
}
