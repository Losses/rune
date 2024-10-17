import 'package:flutter/services.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/navigation/controller_intent.dart';
import '../../config/navigation.dart';
import '../../widgets/playback_controller/constants/controller_items.dart';

import 'back_intent.dart';
import 'escape_intent.dart';
import 'navigation_item.dart';
import 'navigation_intent.dart';

Map<SingleActivator, Intent> buildShortcuts() {
  final shortcuts = <SingleActivator, Intent>{
    const SingleActivator(LogicalKeyboardKey.space): const PrioritizedIntents(
      orderedIntents: <Intent>[
        ActivateIntent(),
        ScrollIntent(
            direction: AxisDirection.down, type: ScrollIncrementType.page),
      ],
    ),
    // On the web, enter activates buttons, but not other controls.
    const SingleActivator(LogicalKeyboardKey.enter):
        const ButtonActivateIntent(),
    const SingleActivator(LogicalKeyboardKey.numpadEnter):
        const ButtonActivateIntent(),

    // Dismissal
    // const SingleActivator(LogicalKeyboardKey.escape): const DismissIntent(),

    // Keyboard traversal.
    const SingleActivator(LogicalKeyboardKey.tab): const NextFocusIntent(),
    const SingleActivator(LogicalKeyboardKey.tab, shift: true):
        const PreviousFocusIntent(),

    // Scrolling
    const SingleActivator(LogicalKeyboardKey.arrowUp):
        const ScrollIntent(direction: AxisDirection.up),
    const SingleActivator(LogicalKeyboardKey.arrowDown):
        const ScrollIntent(direction: AxisDirection.down),
    const SingleActivator(LogicalKeyboardKey.arrowLeft):
        const ScrollIntent(direction: AxisDirection.left),
    const SingleActivator(LogicalKeyboardKey.arrowRight):
        const ScrollIntent(direction: AxisDirection.right),
    const SingleActivator(LogicalKeyboardKey.pageUp): const ScrollIntent(
        direction: AxisDirection.up, type: ScrollIncrementType.page),
    const SingleActivator(LogicalKeyboardKey.pageDown): const ScrollIntent(
        direction: AxisDirection.down, type: ScrollIncrementType.page),

    // Rune specific logic
    const SingleActivator(LogicalKeyboardKey.goBack): const BackIntent(),
    const SingleActivator(LogicalKeyboardKey.escape): const EscapeIntent(),
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
