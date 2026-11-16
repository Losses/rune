import 'package:fluent_ui/fluent_ui.dart';

import '../../main.dart';
import '../../widgets/router/rune_with_navigation_bar_and_playback_controllor.dart';
import '../../providers/router_path.dart';

import '../dialogs/unavailable_dialog_on_band.dart';
import 'history.dart';
import 'modal_route_entry.dart';
import 'route_entry.dart';
import 'modal_route_wrapper.dart';
import 'router_transition_parameter.dart';

NavigatorState $state() {
  return runeWithNavigationBarAndPlaybackControllorNavigatorKey.currentState!;
}

BuildContext $context() {
  return runeWithNavigationBarAndPlaybackControllorNavigatorKey.currentContext!;
}

bool get $hasLibraryNavigator =>
    runeWithNavigationBarAndPlaybackControllorNavigatorKey.currentState != null;

Object? $arguments() {
  return $router.parameter;
}

bool $canPop() {
  return $history.history.length > 1;
}

bool $pop() {
  final navigation = $state();

  if (!$canPop()) return false;

  final current = $history.current;

  if (current == null) return false;

  if (current is ModalRouteEntry) {
    current.pop();
    $history.pop();
    return true;
  }

  if (current is RouteEntry) {
    final (success, result) = $history.pop();

    final next = $history.current;

    if (success) {
      $router.update(next!.name, next.arguments);
      navigation.pop();
      return true;
    }
  }

  return false;
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
  RouteTransitionsBuilder? transitionBuilder,
  bool staysUsableWhenShrunk = true,
}) {
  // A dialog already open when the window shrinks stays mounted and scrolls,
  // so what the user typed survives. Dialogs with their own answer to a small
  // layout ([UnavailableDialogOnBand], [RemoveDialogOnBand]) opt out.
  Widget wrapped(BuildContext context) => ModalRouteWrapper(
    name: name ?? 'modal',
    arguments: arguments,
    canPop: canPop,
    builder: staysUsableWhenShrunk
        ? (context, close) =>
              ScrollableDialogFrame(child: builder(context, close))
        : builder,
  );
  // Fluent's own transition when none is given.
  if (transitionBuilder == null) {
    return showDialog<T>(
      context: context,
      barrierDismissible: barrierDismissible,
      dismissWithEsc: dismissWithEsc,
      builder: wrapped,
    );
  }
  return showDialog<T>(
    context: context,
    barrierDismissible: barrierDismissible,
    dismissWithEsc: dismissWithEsc,
    transitionBuilder: transitionBuilder,
    builder: wrapped,
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
