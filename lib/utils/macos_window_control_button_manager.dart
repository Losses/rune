import 'package:flutter/services.dart';

class MacOSWindowControlButtonManager {
  static const platform = MethodChannel('not.ci.rune/window_control_button');

  static Future<void> setVertical() async {
    await platform.invokeMethod('set_vertical');
  }
}
