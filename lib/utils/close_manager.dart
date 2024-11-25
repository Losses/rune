import 'package:bitsdojo_window/bitsdojo_window.dart';
import 'package:flutter_window_close/flutter_window_close.dart';

class CloseManager {
  bool forceClose = false;

  CloseManager() {
    FlutterWindowClose.setWindowShouldCloseHandler(() async {
      if (forceClose) return true;

      appWindow.hide();
      return false;
    });
  }

  close() {
    forceClose = true;
    appWindow.close();
  }
}

final $closeManager = CloseManager();
