import 'dart:io';

import 'package:flutter/scheduler.dart';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:system_tray/system_tray.dart';
import 'package:bitsdojo_window/bitsdojo_window.dart';

import '../providers/status.dart';
import '../providers/router_path.dart';

import 'api/play_next.dart';
import 'api/play_pause.dart';
import 'api/play_play.dart';
import 'api/play_previous.dart';
import 'close_manager.dart';
import 'l10n.dart';

final SystemTray systemTray = SystemTray();

class TrayManager {
  static String getTrayIconPath() {
    if (Platform.isMacOS) {
      return 'assets/mac-tray.svg';
    }

    final brightness =
        SchedulerBinding.instance.platformDispatcher.platformBrightness ==
                Brightness.light
            ? Brightness.dark.name
            : Brightness.light.name;

    if (Platform.isWindows) {
      return 'assets/tray_icon_$brightness.ico';
    }

    if (Platform.isLinux && bool.hasEnvironment('FLATPAK_ID')) {
      return 'ci.not.Rune-tray-$brightness';
    }

    return 'assets/linux-tray-$brightness.svg';
  }

  bool? _cachedPlaying;
  String? _cachedLocale;

  Future<void> updateTray(BuildContext context) async {
    final updated = await updateTrayItem(context);
    if (updated) {
      await updateTrayIcon();
    }
  }

  Future<bool> updateTrayItem(BuildContext context) async {
    final path = $router.path;
    final status = Provider.of<PlaybackStatusProvider>(context, listen: false);
    final bool playing =
        !status.notReady && status.playbackStatus.state == "Playing";

    final s = S.of(context);
    final locale = s.localeName;

    final suppressRefresh =
        playing == _cachedPlaying && locale == _cachedLocale;

    if (suppressRefresh) return false;

    _cachedPlaying = playing;
    _cachedLocale = locale;

    $closeManager.notificationTitle = s.closeNotification;
    $closeManager.notificationSubtitle = s.closeNotificationSubtitle;

    final menuItems = [
      MenuItemLabel(
        label: s.showRune,
        onClicked: (_) => appWindow.show(),
      ),
      MenuSeparator(),
      if (!status.notReady && path != '/' && path != '/scanning') ...[
        MenuItemLabel(
          label: s.previous,
          onClicked: (_) => _handlePrevious(),
        ),
        MenuItemLabel(
          label: playing ? s.pause : s.play,
          onClicked: (_) => _handlePlayPause(playing),
        ),
        MenuItemLabel(
          label: s.next,
          onClicked: (_) => _handleNext(),
        ),
        MenuSeparator(),
      ],
      MenuItemLabel(
        label: s.exit,
        onClicked: (_) => $closeManager.close(),
      ),
    ];

    final menu = Menu();
    await menu.buildFrom(menuItems);
    await systemTray.setContextMenu(menu);

    return true;
  }

  Future<void> updateTrayIcon() async {
    final iconPath = getTrayIconPath();
    await systemTray.setImage(iconPath, isTemplate: true);
  }

  static void registerEventHandlers() {
    systemTray.registerSystemTrayEventHandler((eventName) {
      if (eventName == kSystemTrayEventClick) {
        Platform.isWindows ? appWindow.show() : systemTray.popUpContextMenu();
      } else if (eventName == kSystemTrayEventRightClick) {
        Platform.isWindows ? systemTray.popUpContextMenu() : appWindow.show();
      }
    });
  }

  void _handlePrevious() {
    playPrevious();
  }

  void _handlePlayPause(bool isPlaying) {
    if (isPlaying) {
      playPause();
    } else {
      playPlay();
    }
  }

  void _handleNext() {
    playNext();
  }
}

final $tray = TrayManager();
