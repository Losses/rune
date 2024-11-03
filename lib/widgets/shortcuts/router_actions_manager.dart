import 'package:fluent_ui/fluent_ui.dart';
import 'package:rune/utils/navigation/utils/navigation_backward.dart';

import '../../utils/navigation/back_action.dart';
import '../../utils/navigation/back_intent.dart';
import '../../utils/navigation/escape_action.dart';
import '../../utils/navigation/escape_intent.dart';
import '../../utils/navigation/navigation_action.dart';
import '../../utils/navigation/navigation_intent.dart';
import '../../utils/navigation/controller_action.dart';
import '../../utils/navigation/controller_intent.dart';
import '../navigation_mouse_key_listener.dart';

class NavigationShortcutManager extends StatelessWidget {
  const NavigationShortcutManager(this.child, {super.key});

  final Widget child;

  @override
  Widget build(BuildContext context) {
    return Actions(
      actions: <Type, Action<Intent>>{
        NavigationIntent: NavigationAction(),
        ControllerIntent: ControllerAction(context),
        EscapeIntent: EscapeAction(),
        BackIntent: BackAction(),
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
