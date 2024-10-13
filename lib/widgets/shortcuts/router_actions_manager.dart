import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/navigation/back_action.dart';
import '../../utils/navigation/back_intent.dart';
import '../../utils/navigation/escape_action.dart';
import '../../utils/navigation/escape_intent.dart';
import '../../utils/navigation/navigation_action.dart';
import '../../utils/navigation/navigation_intent.dart';
import '../../utils/navigation/controller_action.dart';
import '../../utils/navigation/controller_intent.dart';
import '../../utils/navigation/utils/navigation_backward.dart';

class NavigationShortcutManager extends StatelessWidget {
  const NavigationShortcutManager({
    super.key,
    required this.child,
  });

  final Widget child;

  @override
  Widget build(BuildContext context) {
    return BackButtonListener(
      onBackButtonPressed: () async {
        return navigationBackward(context);
      },
      child: Actions(
        actions: <Type, Action<Intent>>{
          NavigationIntent: NavigationAction(context),
          ControllerIntent: ControllerAction(context),
          EscapeIntent: EscapeAction(context),
          BackIntent: BackAction(context),
        },
        child: child,
      ),
    );
  }
}
