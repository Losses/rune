import 'package:flutter/services.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/navigation/back_action.dart';
import '../../utils/navigation/back_intent.dart';
import '../../utils/navigation/escape_action.dart';
import '../../utils/navigation/navigation_action.dart';
import '../../utils/navigation/navigation_intent.dart';
import '../../utils/navigation/controller_action.dart';
import '../../utils/navigation/controller_intent.dart';
import '../../utils/navigation/open_action.dart';
import '../../utils/navigation/open_intent.dart';
import '../../utils/navigation/utils/navigation_backward.dart';

import '../navigation_mouse_key_listener.dart';

class NavigationShortcutManager extends StatelessWidget {
  const NavigationShortcutManager(this.child, {super.key});
  // Method channel for Android back button
  static const MethodChannel popChannel = MethodChannel('ci.not.rune/pop');

  final Widget child;

  @override
  Widget build(BuildContext context) {
    popChannel.setMethodCallHandler((call) async {
      // Handle back button press on Android
      if (call.method == 'pop') {
        return navigationBackward();
      }
    });
    return Actions(
      actions: <Type, Action<Intent>>{
        NavigationIntent: NavigationAction(),
        ControllerIntent: ControllerAction(context),
        DismissIntent: EscapeAction(),
        BackIntent: BackAction(),
        OpenIntent: OpenAction(),
      },
      child: NavigationMouseKeyListener(
        onBackwardMouseButtonTapDown: (_) {
          navigationBackward();
        },
        child: child,
      ),
    );
  }
}
