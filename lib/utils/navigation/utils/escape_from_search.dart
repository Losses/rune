import 'package:fluent_ui/fluent_ui.dart';
import 'package:go_router/go_router.dart';

escapeFromSearch(BuildContext context) {
  if (GoRouterState.of(context).fullPath == '/search') {
    if (context.canPop()) {
      context.pop();
    } else {
      context.go('/library');
    }

    return true;
  }

  return false;
}
