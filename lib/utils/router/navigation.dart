import 'package:fluent_ui/fluent_ui.dart';

import '../../main.dart';
import '../../widgets/router/rune_with_navigation_bar_and_playback_controllor.dart';
import '../../providers/router_path.dart';

import 'history.dart';
import 'route_entry.dart';
import 'modal_route_wrapper.dart';
import 'router_transition_parameter.dart';

NavigatorState $state() {
  return runeWithNavigationBarAndPlaybackControllorNavigatorKey.currentState!;
}

BuildContext $context() {
  return runeWithNavigationBarAndPlaybackControllorNavigatorKey.currentContext!;
}

Object? $arguments() {
  return $router.parameter;
}

bool $canPop() {
  return $history.history.length > 1;
}

bool $pop() {
  final navigation = $state();
  if ($canPop()) {
    final (canPop, result) = $history.pop();
    if (!canPop) return false;

    final current = $history.current;
    if (current != null) {
      if (current is RouteEntry) {
        final settings = current.toSettings();
        $router.update(settings.name, settings.arguments);
      }
      navigation.pop(result);
    }
    return true;
  }
  return false;
}

Future<bool> $popModal() async {
  if (!$history.isCurrentModal) {
    return false;
  }

  final (canPop, result) = $history.pop();
  if (canPop) {
    final navigation = $state();
    navigation.pop(result);
  }
  return true;
}

Future<T?> $showModal<T extends Object?>(
  BuildContext context,
  Widget Function(BuildContext context, void Function(T? result) close)
      builder, {
  String? name,
  Object? arguments,
  (bool, dynamic) Function()? canPop,
  bool barrierDismissible = false,
  bool dismissWithEsc = false,
}) {
  return showDialog<T>(
    context: context,
    barrierDismissible: barrierDismissible,
    dismissWithEsc: dismissWithEsc,
    builder: (context) => ModalRouteWrapper(
      name: name ?? 'modal',
      arguments: arguments,
      canPop: canPop,
      builder: builder,
    ),
  );
}

Future<T?>? $push<T extends Object?>(
  String routeName, {
  Object? arguments,
}) {
  final navigation = $state();

  final from = $router.path ?? "/library";
  final to = routeName;

  final p = RouterTransitionParameter(from, to, arguments);

  $router.update(routeName, p);

  final newSettings = RouteSettings(name: routeName, arguments: p);
  $history.push(newSettings);

  return navigation.pushNamed(routeName, arguments: p);
}

Future<T?>? $replace<T extends Object?>(
  String routeName, {
  Object? arguments,
}) {
  final navigation = $state();

  final from = $router.path ?? "/library";
  final to = routeName;

  final p = RouterTransitionParameter(from, to, arguments);

  $router.update(routeName, p);

  final newSettings = RouteSettings(name: routeName, arguments: p);
  $history.replace(newSettings);

  return navigation.pushReplacementNamed(routeName, arguments: p);
}

NavigatorState $$state() {
  return rootNavigatorKey.currentState!;
}

Future<T?>? $$replace<T extends Object?>(
  String routeName, {
  Object? arguments,
}) {
  final navigation = $$state();

  $history.reset();
  $router.update("/library", null);
  return navigation.pushReplacementNamed(routeName);
}
