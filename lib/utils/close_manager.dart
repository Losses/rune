import 'dart:io';

import 'package:bitsdojo_window/bitsdojo_window.dart';
import 'package:flutter/services.dart';
import 'package:local_notifier/local_notifier.dart';
import 'package:flutter_window_close/flutter_window_close.dart';

import 'settings_manager.dart';

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
        if (Platform.isWindows || Platform.isLinux) {
          final LocalNotification notification = LocalNotification(
            title: notificationTitle ?? "",
            body: notificationSubtitle ?? "",
          );

          notification.show();

          SettingsManager().setValue<bool>(closeNotificationShownKey, true);
        }
      }

      return false;
    });
  }

  close() {
    forceClose = true;
    
    if (Platform.isMacOS) {
      SystemNavigator.pop();
    } else {
      appWindow.close();
    }
  }
}

final $closeManager = CloseManager();
