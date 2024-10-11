import 'package:fluent_ui/fluent_ui.dart';

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
      child: child,
    );
  }
}
