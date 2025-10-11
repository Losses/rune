import 'dart:io';
import 'package:flutter/services.dart';
import 'package:flutter/foundation.dart';

class LinuxWindowFrameManager {
  static final shared = LinuxWindowFrameManager._();

  LinuxWindowFrameManager._();

  final platform = MethodChannel('not.ci.rune/linux_window_frame');

  Future<void> setCustomFrame(bool enabled) async {
    if (!Platform.isLinux) return;

    try {
      // Call native method to set custom frame
      await platform.invokeMethod('set_custom_frame', {'enabled': enabled});
    } catch (e) {
      // For now, just log the change - the native implementation needs to be fixed
      if (kDebugMode) {
        print('Frame setting updated to: ${enabled ? "custom" : "native"} frame');
      }
    }
  }
}