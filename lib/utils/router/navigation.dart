import 'package:fluent_ui/fluent_ui.dart';

import '../../providers/router_path.dart';
import '../../utils/router/router_transition_parameter.dart';
import '../../widgets/router/rune_with_navigation_bar_and_playback_controllor.dart';

bool $canPop() {
  final navigation =
      runeWithNavigationBarAndPlaybackControllorNavigatorKey.currentState!;

  return navigation.canPop();
}

bool $pop() {
  final navigation =
      runeWithNavigationBarAndPlaybackControllorNavigatorKey.currentState!;

  if (navigation.canPop()) {
    navigation.pop();
    return true;
  }

  return false;
}

Future<T?>? $push<T extends Object?>(
  BuildContext context,
  String routeName, {
  Object? arguments,
}) {
  final navigation =
      runeWithNavigationBarAndPlaybackControllorNavigatorKey.currentState;

  final from = ModalRoute.of(context)?.settings.name ?? "/";
  final to = routeName;

  $routerPath.update(routeName);

  return navigation?.pushNamed(
    routeName,
    arguments: RouterTransitionParameter(from, to, arguments),
  );
}

Future<T?>? $replace<T extends Object?>(
  BuildContext context,
  String routeName, {
  Object? arguments,
}) {
  final navigation =
      runeWithNavigationBarAndPlaybackControllorNavigatorKey.currentState;

  final from = ModalRoute.of(context)?.settings.name ?? "/";
  final to = routeName;

  $routerPath.update(routeName);

  return navigation?.pushReplacementNamed(
    routeName,
    arguments: RouterTransitionParameter(from, to, arguments),
  );
}
