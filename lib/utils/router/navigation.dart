import 'package:fluent_ui/fluent_ui.dart';

import '../../providers/router_path.dart';
import '../../utils/router/router_transition_parameter.dart';
import '../../widgets/router/rune_with_navigation_bar_and_playback_controllor.dart';

Object? $arguments() {
  return $router.parameter;
}

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

    final settings = ModalRoute.of(
            runeWithNavigationBarAndPlaybackControllorNavigatorKey
                .currentContext!)!
        .settings;

    final path = settings.name;
    final parameter = settings.arguments;

    $router.update(path, parameter);

    return true;
  }

  return false;
}

Future<T?>? $push<T extends Object?>(
  String routeName, {
  Object? arguments,
}) {
  final navigation =
      runeWithNavigationBarAndPlaybackControllorNavigatorKey.currentState;

  final from = $router.path ?? "/library";
  final to = routeName;

  final p = RouterTransitionParameter(from, to, arguments);

  $router.update(routeName, p);

  return navigation!.pushNamed(routeName, arguments: p);
}

Future<T?>? $replace<T extends Object?>(
  String routeName, {
  Object? arguments,
}) {
  final navigation =
      runeWithNavigationBarAndPlaybackControllorNavigatorKey.currentState;

  final from = $router.path ?? "/library";
  final to = routeName;

  final p = RouterTransitionParameter(from, to, arguments);

  $router.update(routeName, p);

  return navigation!.pushReplacementNamed(routeName, arguments: p);
}
