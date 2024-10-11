import 'package:flutter/services.dart';

import 'package:fluent_ui/fluent_ui.dart';

import 'back_intent.dart';
import 'escape_intent.dart';
import 'navigation_item.dart';
import 'navigation_intent.dart';

Map<LogicalKeySet, Intent> buildShortcuts(List<NavigationItem> items) {
  final shortcuts = <LogicalKeySet, Intent>{
    LogicalKeySet(LogicalKeyboardKey.goBack): const BackIntent(),
    LogicalKeySet(LogicalKeyboardKey.escape): const EscapeIntent(),
    LogicalKeySet(LogicalKeyboardKey.backspace): const BackIntent(),
  };

  void addShortcuts(List<NavigationItem> items) {
    for (var item in items) {
      if (item.shortcuts != null) {
        for (var keySet in item.shortcuts!) {
          shortcuts[keySet] = NavigationIntent(item.path);
        }
      }
      if (item.children != null && item.children!.isNotEmpty) {
        addShortcuts(item.children!);
      }
    }
  }

  addShortcuts(items);

  return shortcuts;
}
