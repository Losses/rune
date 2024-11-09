import 'package:flutter/services.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/navigation/controller_intent.dart';
import '../../config/navigation.dart';
import '../../widgets/playback_controller/constants/controller_items.dart';

import 'back_intent.dart';
import 'navigation_item.dart';
import 'navigation_intent.dart';

Map<SingleActivator, Intent> buildShortcuts() {
  final shortcuts = <SingleActivator, Intent>{
    const SingleActivator(LogicalKeyboardKey.goBack): const BackIntent(),
    const SingleActivator(LogicalKeyboardKey.escape): const DismissIntent(),
    const SingleActivator(LogicalKeyboardKey.backspace): const BackIntent(),
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

Map<SingleActivator, Intent> buildNoShortcuts(Map<SingleActivator, Intent> x) {
  final Map<SingleActivator, Intent> result = {};

  for (final entry in x.entries) {
    result[entry.key] = const DoNothingAndStopPropagationTextIntent();
  }

  result[const SingleActivator(LogicalKeyboardKey.backspace)] =
      const DeleteCharacterIntent(forward: false);
  result[const SingleActivator(LogicalKeyboardKey.escape)] =
      const DismissIntent();

  return result;
}
