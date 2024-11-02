import 'package:fluent_ui/fluent_ui.dart';

import 'navigation_intent.dart';

class NavigationAction extends Action<NavigationIntent> {
  final BuildContext context;

  NavigationAction(this.context);

  @override
  void invoke(covariant NavigationIntent intent) {
    final currentPath = ModalRoute.of(context)?.settings.name;
    if (intent.path == currentPath) {
      return;
    }

    Navigator.pushNamed(context, intent.path);
  }
}
