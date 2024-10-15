import 'package:fluent_ui/fluent_ui.dart';
import 'package:go_router/go_router.dart';
import 'package:rune/config/navigation_query.dart';

bool navigationBackward(BuildContext context) {
  final router = GoRouter.of(context);
  final canPop = router.canPop();

  if (!canPop) {
    final routerState = GoRouterState.of(context);
    final path = routerState.fullPath;
    final parent = navigationQuery.getParent(path, false);
    if (parent != null && parent.path != '/' && parent.path != '/home') {
      router.go(parent.path);
    }
  }

  return !canPop;
}

navigateBackwardWithPop(BuildContext context) {
  final router = GoRouter.of(context);
  final routerState = GoRouterState.of(context);

  if (!navigationBackward(context)) {
    router.pop();
  } else if (routerState.path != '/library') {
    router.go('/library');
  }
}
