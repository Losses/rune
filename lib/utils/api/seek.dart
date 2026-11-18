import 'dart:async';

import '../../bindings/bindings.dart';
import '../../providers/status.dart';
import 'seek_absolute.dart';

bool _shouldExecute = true;

void seek(double value, PlaybackStatusState? status) {
  if (status == null || !_shouldExecute) return;
  if (!seekAbsolute((value / 100) * status.duration, status)) return;

  _shouldExecute = false;
  Timer(const Duration(milliseconds: 42), () {
    _shouldExecute = true;
  });
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
