import 'package:go_router/go_router.dart';
import 'package:fluent_ui/fluent_ui.dart';

import 'navigation_intent.dart';

class NavigationAction extends Action<NavigationIntent> {
  final BuildContext context;

  NavigationAction(this.context);

  @override
  void invoke(covariant NavigationIntent intent) {
    final currentPath = GoRouterState.of(context).fullPath;
    if (intent.path == currentPath) {
      return;
    }

    GoRouter.of(context).push(intent.path);
  }
}
