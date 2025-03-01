import 'dart:io';

import 'package:flutter/services.dart';
import 'package:local_notifier/local_notifier.dart';
import 'package:bitsdojo_window/bitsdojo_window.dart';
import 'package:flutter_window_close/flutter_window_close.dart';

import '../constants/configurations.dart';
import 'settings_manager.dart';

final settingsManager = SettingsManager();

class CloseManager {
  bool forceClose = false;

  String? notificationTitle;
  String? notificationSubtitle;

  CloseManager() {
    FlutterWindowClose.setWindowShouldCloseHandler(() async {
      if (forceClose) return true;

      final closingWindowBehavior =
          await settingsManager.getValue<String>(kClosingWindowBehaviorKey);
      if (closingWindowBehavior == "exit") {
        return true;
      }

      appWindow.hide();

      final closeNotificationShown =
          await settingsManager.getValue<bool>(kCloseNotificationShownKey);

      if (closeNotificationShown != true) {
        if (Platform.isWindows || Platform.isLinux) {
          final LocalNotification notification = LocalNotification(
            title: notificationTitle ?? "",
            body: notificationSubtitle ?? "",
          );

          notification.show();

          SettingsManager().setValue<bool>(kCloseNotificationShownKey, true);
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
