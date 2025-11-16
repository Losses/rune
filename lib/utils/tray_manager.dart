import 'dart:io';

import 'package:flutter/scheduler.dart';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:system_tray/system_tray.dart';
import 'package:bitsdojo_window/bitsdojo_window.dart';

import '../providers/status.dart';
import '../providers/router_path.dart';

import '../constants/configurations.dart';
import '../constants/settings_manager.dart';

import 'api/play_next.dart';
import 'api/play_pause.dart';
import 'api/play_play.dart';
import 'api/play_previous.dart';
import 'close_manager.dart';
import 'l10n.dart';

final SystemTray systemTray = SystemTray();

class TrayIcon {
  final String path;
  final bool isInstalled;

  TrayIcon(this.path, this.isInstalled);
}

class TrayManager {
  static Future<TrayIcon> getTrayIcon() async {
    if (Platform.isMacOS) {
      return TrayIcon('assets/mac-tray.svg', false);
    }

    // Get user preference for tray icon color mode
    final trayIconColorMode =
        await $settingsManager.getValue<String>(kTrayIconColorModeKey) ??
            "auto";

    String brightness;
    if (trayIconColorMode == "auto") {
      // Automatic mode: follow system theme
      // Special case: GNOME always uses light icon
      if (Platform.isLinux &&
          Platform.environment['XDG_CURRENT_DESKTOP'] == 'GNOME') {
        brightness = Brightness.light.name;
      } else {
        brightness =
            SchedulerBinding.instance.platformDispatcher.platformBrightness ==
                    Brightness.light
                ? Brightness.dark.name
                : Brightness.light.name;
      }
    } else {
      // Manual mode: use user's selection directly
      brightness = trayIconColorMode;
    }

    if (Platform.isWindows) {
      return TrayIcon('assets/tray_icon_$brightness.ico', false);
    }

    if (Platform.isLinux && Platform.environment.containsKey('FLATPAK_ID')) {
      return TrayIcon('ci.not.Rune-tray-$brightness', true);
    }

    return TrayIcon('assets/linux-tray-$brightness.svg', false);
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
    final icon = await TrayManager.getTrayIcon();
    await systemTray.setImage(icon.path, isTemplate: true, isInstalled: icon.isInstalled);
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
