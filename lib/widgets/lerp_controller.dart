import 'package:flutter/scheduler.dart';

class LerpController {
  late double _value;
  double Function() getter;
  void Function(double) setter;
  Ticker? _ticker;

  LerpController(initialValue, this.getter, this.setter, TickerProvider vsync) {
    _ticker = vsync.createTicker(_onTick);
    _value = 0;
  }

  void dispose() {
    _ticker?.dispose();
  }

  void lerp(double value) {
    _value = value;

    if (_ticker?.isTicking == false) {
      _ticker?.start();
    }
  }

  void _onTick(Duration elapsed) {
    final currentValue = getter();
    if ((currentValue - _value).abs() < 1e-1) {
      _ticker?.stop();
    } else {
      double actualValue = lerpDouble(
        currentValue,
        _value,
        0.1,
      )!;
      setter(actualValue);
    }
  }

  double? lerpDouble(double a, double b, double t) {
    return a + (b - a) * t;
  }

  double get value => _value;
}
