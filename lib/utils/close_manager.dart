import 'dart:io';

import 'package:bitsdojo_window/bitsdojo_window.dart';
import 'package:flutter_window_close/flutter_window_close.dart';

import 'settings_manager.dart';
import 'windows_notification.dart';

final closeNotificationShownKey = 'close_notification_shown';

class CloseManager {
  bool forceClose = false;

  String? notificationTitle;
  String? notificationSubtitle;

  CloseManager() {
    FlutterWindowClose.setWindowShouldCloseHandler(() async {
      if (forceClose) return true;

      appWindow.hide();

      if (await SettingsManager().getValue<bool>(closeNotificationShownKey) !=
          true) {
        if (Platform.isWindows) {
          showNotification(notificationTitle ?? '', notificationSubtitle ?? '');
        }
      }

      return false;
    });
  }

  close() {
    forceClose = true;
    appWindow.close();
  }
}

final $closeManager = CloseManager();
