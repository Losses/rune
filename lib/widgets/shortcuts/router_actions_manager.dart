import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/navigation/back_action.dart';
import '../../utils/navigation/back_intent.dart';
import '../../utils/navigation/escape_action.dart';
import '../../utils/navigation/escape_intent.dart';
import '../../utils/navigation/navigation_action.dart';
import '../../utils/navigation/navigation_intent.dart';
import '../../utils/navigation/controller_action.dart';
import '../../utils/navigation/controller_intent.dart';

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
      child: child,
    );
  }
}
