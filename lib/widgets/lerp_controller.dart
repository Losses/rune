import 'package:flutter/scheduler.dart';
import 'dart:async';

class LerpController {
  late double _value;
  double Function() getter;
  void Function(double) setter;
  Ticker? _ticker;
  Completer<void>? _completer;

  LerpController(initialValue, this.getter, this.setter, TickerProvider vsync) {
    _ticker = vsync.createTicker(_onTick);
    _value = initialValue;
  }

  void dispose() {
    _ticker?.dispose();
  }

  Future<void> lerp(double value) {
    if (_completer != null && !_completer!.isCompleted) {
      _completer!.complete();
    }

    _value = value;
    _completer = Completer<void>();

    if (_ticker?.isTicking == false) {
      _ticker?.start();
    }

    return _completer!.future;
  }

  void _onTick(Duration elapsed) {
    final currentValue = getter();
    if ((currentValue - _value).abs() < 1e-2) {
      _ticker?.stop();
      _completer?.complete();
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
