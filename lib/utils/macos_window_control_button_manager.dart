import 'package:flutter/services.dart';

class MacOSWindowControlButtonManager {
  static var shared = MacOSWindowControlButtonManager._();

  MacOSWindowControlButtonManager._();

  var platform = MethodChannel('not.ci.rune/window_control_button');

  Future<void> setVertical() async {
    await platform.invokeMethod('set_vertical');
  }

  Future<void> setHide() async {
    await platform.invokeMethod('set_hide');
  }

  Future<void> setShow() async {
    await platform.invokeMethod('set_show');
  }
}
