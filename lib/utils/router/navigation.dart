import 'package:fluent_ui/fluent_ui.dart';

import '../../main.dart';
import '../../widgets/router/rune_with_navigation_bar_and_playback_controllor.dart';
import '../../providers/router_path.dart';

import '../dialogs/unavailable_dialog_on_band.dart';
import 'context_menu_route_entry.dart';
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
  if (!$canPop()) return false;

  final current = $history.current;

  if (current == null) return false;

  if (current is ModalRouteEntry) {
    // The modal checks canPop before removing its own entry.
    current.pop();
    return true;
  }

  // A context menu open over the page closes, as a dialog does.
  if (current is ContextMenuRouteEntry) {
    $history.remove(current);
    current.pop();
    return true;
  }

  // Pages live in the library's navigator; the welcome screens, which can
  // show dialogs, have none.
  if (current is RouteEntry && $hasLibraryNavigator) {
    final (success, result) = $history.pop();
    // Menus left under the page go with it.
    _closeContextMenus();

    final next = $history.currentPage;

    if (success && next != null) {
      $router.update(
        next.name,
        popTransitionParameter(current.name, next.name, next.arguments),
      );
      $state().pop();
      return true;
    }
  }

  return false;
}

/// Goes back to the page [name] below the current one, closing every page above
/// it at once, as [$pop] goes back one page. Returns false and does nothing
/// when no page [name] is below the current one.
bool $popTo(String name) {
  final current = $history.current;
  if (current is! RouteEntry) return false;

  final revealed = $history.popTo(name);
  if (revealed == null) return false;

  $router.update(
    revealed.name,
    popTransitionParameter(current.name, revealed.name, revealed.arguments),
  );
  $state().popUntil((route) => route.settings.name == name);
  return true;
}

/// The pages that are only ever open once. Opening one that is already open
/// below goes back to it, as it was left, instead of opening another on top.
const _openOnce = {'/search'};

/// The router's parameter for revealing [revealedName] again after popping
/// [poppedName]: a transition from the popped page to the revealed one, around
/// the revealed entry's own [arguments], which pages keep reading as before.
RouterTransitionParameter popTransitionParameter(
  String poppedName,
  String revealedName,
  Object? arguments,
) {
  final inner = arguments is RouterTransitionParameter
      ? arguments.parameters
      : arguments;
  return RouterTransitionParameter(poppedName, revealedName, inner);
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
  bool useRootNavigator = true,
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
      useRootNavigator: useRootNavigator,
      builder: wrapped,
    );
  }
  return showDialog<T>(
    context: context,
    barrierDismissible: barrierDismissible,
    dismissWithEsc: dismissWithEsc,
    transitionBuilder: transitionBuilder,
    useRootNavigator: useRootNavigator,
    builder: wrapped,
  );
}

/// Closes the context menus open over the current page, as a menu item that
/// opens a page does, before the page is pushed. A menu closes itself only
/// after its item ran, which leaves its route and its history entry under the
/// new page, so going back would reveal the menu instead of the page below.
void _closeContextMenus() {
  for (var top = $history.current; top is ContextMenuRouteEntry;) {
    $history.remove(top);
    top.pop();
    top = $history.current;
  }
}

Future<T?>? $push<T extends Object?>(String routeName, {Object? arguments}) {
  _closeContextMenus();

  if (_openOnce.contains(routeName) && $popTo(routeName)) return null;

  final navigation = $state();

  final from = $router.path ?? "/library";
  final to = routeName;

  final p = RouterTransitionParameter(from, to, arguments);

  $router.update(routeName, p);

  final newSettings = RouteSettings(name: routeName, arguments: p);
  $history.push(newSettings);

  return navigation.pushNamed(routeName, arguments: p);
}

Future<T?>? $replace<T extends Object?>(String routeName, {Object? arguments}) {
  _closeContextMenus();

  final navigation = $state();

  final from = $router.path ?? "/library";
  final to = routeName;

  final p = RouterTransitionParameter(from, to, arguments);

  $router.update(routeName, p);

  final newSettings = RouteSettings(name: routeName, arguments: p);
  $history.replace(newSettings);

  return navigation.pushReplacementNamed(routeName, arguments: p);
}

/// Opens the page [routeName] that the user jumped to from a menu, as Go to
/// Album and Go to Artist do. A page opened from search stays one level under
/// Search, so a page reached from it replaces it instead of stacking on it, and
/// going back leads to search. Anywhere else the page is pushed on the current
/// one, and going back leads to the page the user came from.
Future<T?>? $goTo<T extends Object?>(String routeName, {Object? arguments}) {
  final fromSearch =
      $router.path != '/search' && $history.previousPageIs('/search');
  return fromSearch
      ? $replace<T>(routeName, arguments: arguments)
      : $push<T>(routeName, arguments: arguments);
}

/// Whether the root navigator is currently mounted.
bool get $$hasRootNavigator => rootNavigatorKey.currentState != null;

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
