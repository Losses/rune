import 'dart:async';

import 'package:flutter/scheduler.dart';

class LerpController {
  late double _value;
  double Function() getter;
  void Function(double) setter;
  Ticker? _ticker;
  Completer<void>? _completer;
  double t;

  LerpController({
    required double initialValue,
    required this.getter,
    required this.setter,
    this.t = 0.1,
    required TickerProvider vsync,
  }) {
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
    final delta = (currentValue - _value).abs();
    if (delta < 0.01) {
      _completer?.complete();
      _ticker?.stop();
    } else {
      double actualValue = lerpDouble(
        currentValue,
        _value,
        delta < 0.1 ? t * 2 : t,
      )!;
      setter(actualValue);
    }
  }

  double? lerpDouble(double a, double b, double t) {
    return a + (b - a) * t;
  }

  void syncValue(double value) {
    _value = value;
  }

  bool get isAnimating => _ticker?.isTicking ?? false;

  double get value => _value;
}
