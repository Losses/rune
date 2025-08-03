import 'dart:async';

import '../../bindings/bindings.dart';
import '../../providers/status.dart';

bool _shouldExecute = true;

void seek(double value, PlaybackStatusState? status) {
  if (_shouldExecute) {
    if (status != null) {
      SeekRequest(
        positionSeconds: (value / 100) * (status.duration),
      ).sendSignalToRust();
    }
    _shouldExecute = false;
    Timer(const Duration(milliseconds: 42), () {
      _shouldExecute = true;
    });
  }
}

void seek0(double value, PlaybackStatus? status) {
  if (_shouldExecute) {
    if (status != null) {
      SeekRequest(
        positionSeconds: (value / 100) * (status.duration),
      ).sendSignalToRust();
    }
    _shouldExecute = false;
    Timer(const Duration(milliseconds: 42), () {
      _shouldExecute = true;
    });
  }
}
