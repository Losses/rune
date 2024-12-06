import 'dart:async';

import '../../messages/all.dart';

bool _shouldExecute = true;

void seek(double value, PlaybackStatus? status) {
  if (_shouldExecute) {
    if (status != null) {
      SeekRequest(
        positionSeconds: (value / 100) * status.duration,
      ).sendSignalToRust();
    }
    _shouldExecute = false;
    Timer(const Duration(milliseconds: 42), () {
      _shouldExecute = true;
    });
  }
}
