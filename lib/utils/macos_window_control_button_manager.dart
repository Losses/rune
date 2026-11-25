import 'package:flutter/services.dart';

class MacOSWindowControlButtonManager {
  static final shared = MacOSWindowControlButtonManager._();

  MacOSWindowControlButtonManager._();

  final platform = MethodChannel('not.ci.rune/window_control_button');

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
