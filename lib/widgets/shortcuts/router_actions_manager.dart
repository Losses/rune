import 'package:fluent_ui/fluent_ui.dart';
import 'package:go_router/go_router.dart';

import '../../config/navigation_query.dart';

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
      },
      child: child,
    );
  }
}
