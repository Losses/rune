import 'package:flutter/services.dart';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:player/config/navigation.dart';
import 'package:player/utils/navigation/controller_intent.dart';
import 'package:player/widgets/playback_controller/constants/controller_items.dart';

import 'back_intent.dart';
import 'escape_intent.dart';
import 'navigation_item.dart';
import 'navigation_intent.dart';

Map<LogicalKeySet, Intent> buildShortcuts() {
  final shortcuts = <LogicalKeySet, Intent>{
    LogicalKeySet(LogicalKeyboardKey.goBack): const BackIntent(),
    LogicalKeySet(LogicalKeyboardKey.escape): const EscapeIntent(),
    LogicalKeySet(LogicalKeyboardKey.backspace): const BackIntent(),
  };

  void addNavigationShortcuts(List<NavigationItem> items) {
    for (var item in items) {
      if (item.shortcuts != null) {
        for (var keySet in item.shortcuts!) {
          shortcuts[keySet] = NavigationIntent(item.path);
        }
      }
      if (item.children != null && item.children!.isNotEmpty) {
        addNavigationShortcuts(item.children!);
      }
    }
  }

  addNavigationShortcuts(navigationItems);

  for (var item in controllerItems) {
    if (item.shortcuts != null) {
      for (var keySet in item.shortcuts!) {
        shortcuts[keySet] = ControllerIntent(item);
      }
    }
  }

  return shortcuts;
}
