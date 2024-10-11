import 'package:fluent_ui/fluent_ui.dart';
import 'package:go_router/go_router.dart';
import 'package:player/config/navigation_query.dart';

bool navigationBackward(BuildContext context) {
  final router = GoRouter.of(context);
  final routerState = GoRouterState.of(context);
  final path = routerState.fullPath;

  final parent = navigationQuery.getParent(path, false);

  final canPop = router.canPop();

  if (!canPop) {
    if (parent != null) {
      router.go(parent.path);
    }
  }
  return !canPop;
}
