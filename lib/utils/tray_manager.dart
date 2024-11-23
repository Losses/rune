import 'dart:io';

import 'package:flutter/scheduler.dart';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:tray_manager/tray_manager.dart';

import '../config/theme.dart';
import '../providers/status.dart';
import '../providers/router_path.dart';

import 'l10n.dart';

class TrayManager {
  initialize() async {
    if (Platform.isMacOS) {
      await trayManager.setIcon(getTrayIconPath(), isTemplate: true);
    } else {
      await trayManager.setIcon(getTrayIconPath());
    }
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

    if (Platform.isMacOS) {
      return 'assets/mac-tray.svg';
    }

    return 'assets/linux-tray.svg';
  }

  String? _cachedPath;
  bool? _cachedPlaying;
  Locale? _cachedLocale;

  updateTray(BuildContext context) async {
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

    if (status.notReady || path == '/' || path == '/scanning') {
      final menu = Menu(
        items: [
          MenuItem(
            key: 'show_window',
            label: s.showRune,
          ),
          MenuItem.separator(),
          MenuItem(
            key: 'exit_app',
            label: s.exit,
          ),
        ],
      );

      await trayManager.setContextMenu(menu);
    } else {
      final menu = Menu(
        items: [
          MenuItem(
            key: 'show_window',
            label: s.showRune,
          ),
          MenuItem.separator(),
          MenuItem(
            key: 'previous',
            label: s.previous,
          ),
          playing
              ? MenuItem(
                  key: 'pause',
                  label: s.pause,
                )
              : MenuItem(
                  key: 'play',
                  label: s.play,
                ),
          MenuItem(
            key: 'next',
            label: s.next,
          ),
          MenuItem.separator(),
          MenuItem(
            key: 'exit_app',
            label: s.exit,
          ),
        ],
      );

      await trayManager.setContextMenu(menu);
    }
  }
}

final $tray = TrayManager();
