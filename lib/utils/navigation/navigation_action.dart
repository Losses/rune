import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/router/navigation.dart';
import '../../providers/router_path.dart';

import 'navigation_intent.dart';

class NavigationAction extends Action<NavigationIntent> {
  NavigationAction();

  @override
  void invoke(covariant NavigationIntent intent) {
    final currentPath = $router.path;
    if (intent.path == currentPath) {
      return;
    }

    $push(intent.path);
  }
}
