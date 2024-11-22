import 'dart:io';

import 'package:flutter/scheduler.dart';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:system_tray/system_tray.dart';
import 'package:window_manager/window_manager.dart';

import '../config/theme.dart';
import '../providers/status.dart';
import '../providers/router_path.dart';

import 'api/play_next.dart';
import 'api/play_pause.dart';
import 'api/play_play.dart';
import 'api/play_previous.dart';
import 'l10n.dart';

class TrayManager {
  final SystemTray systemTray = SystemTray();

  TrayManager() {
    if (!Platform.isLinux && !Platform.isWindows) return;

    systemTray.initSystemTray(
      title: "Rune",
      iconPath: getTrayIconPath(),
    );

    systemTray.registerSystemTrayEventHandler((eventName) async {
      if (eventName == kSystemTrayEventClick) {
        Platform.isWindows ? showWindow() : systemTray.popUpContextMenu();
      } else if (eventName == kSystemTrayEventRightClick) {
        Platform.isWindows
            ? systemTray.popUpContextMenu()
            : windowManager.show();
      }
    });
  }

  static String getTrayIconPath() {
    if (Platform.isWindows) {
      if (SchedulerBinding.instance.platformDispatcher.platformBrightness ==
          Brightness.light) {
        return 'assets/tray_icon_dark.ico';
      } else {
        return 'assets/tray_icon_light.ico';
      }
    }

    return 'assets/linux-tray.svg';
  }

  initialize() async {
    if (!Platform.isLinux && !Platform.isWindows) return;

    systemTray.setSystemTrayInfo(
      title: "Rune",
      iconPath: getTrayIconPath(),
    );
  }

  String? _cachedPath;
  bool? _cachedPlaying;
  Locale? _cachedLocale;

  static showWindow() {
    windowManager.setAlwaysOnTop(true);
    windowManager.show();
    windowManager.focus();
    windowManager.restore();
    windowManager.setAlwaysOnTop(false);
  }

  static exit() {
    windowManager.destroy();
  }

  updateTray(BuildContext context) async {
    if (!Platform.isLinux && !Platform.isWindows) return;

    final path = $router.path;

    final s = S.of(context);
    final status = Provider.of<PlaybackStatusProvider>(context, listen: false);
    final bool playing =
        !status.notReady && status.playbackStatus?.state == "Playing";

    final locale = appTheme.locale;
    final suppressRefresh = path == _cachedPath &&
        playing == _cachedPlaying &&
        locale == _cachedLocale;

    if (suppressRefresh) return;

    _cachedPath = path;
    _cachedPlaying = playing;
    _cachedLocale = locale;

    final Menu menu = Menu();
    if (status.notReady || path == '/' || path == '/scanning') {
      menu.buildFrom(
        [
          MenuItemLabel(
            label: s.showRune,
            onClicked: (_) => showWindow(),
          ),
          MenuSeparator(),
          MenuItemLabel(
            label: s.exit,
            onClicked: (_) => exit(),
          ),
        ],
      );
    } else {
      menu.buildFrom(
        [
          MenuItemLabel(
            label: s.showRune,
            onClicked: (_) => showWindow(),
          ),
          MenuSeparator(),
          MenuItemLabel(
            label: s.previous,
            onClicked: (_) => playPrevious(),
          ),
          playing
              ? MenuItemLabel(
                  label: s.pause,
                  onClicked: (_) => playPause(),
                )
              : MenuItemLabel(
                  label: s.play,
                  onClicked: (_) => playPlay(),
                ),
          MenuItemLabel(
            label: s.next,
            onClicked: (_) => playNext(),
          ),
          MenuSeparator(),
          MenuItemLabel(
            label: s.exit,
            onClicked: (_) => exit(),
          ),
        ],
      );
    }

    systemTray.setContextMenu(menu);
  }
}

final $tray = TrayManager();
