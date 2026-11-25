import 'package:fluent_ui/fluent_ui.dart';

import 'context_menu_route_entry.dart';
import 'route_entry.dart';
import 'base_route_entry.dart';
import 'modal_route_entry.dart';
import 'router_transition_parameter.dart';

class NavigationHistory {
  final List<BaseRouteEntry> history = [
    RouteEntry(
      name: "/library",
      arguments: RouterTransitionParameter("/library", "/library"),
    )
  ];

  void push(RouteSettings settings) {
    history.add(RouteEntry.fromSettings(settings));
  }

  void pushModal(ModalRouteEntry entry) {
    history.add(entry);
  }

  void pushContextMenu(ContextMenuRouteEntry entry) {
    history.add(entry);
  }

  reset() {
    history.clear();
    history.add(
      RouteEntry(
        name: "/library",
        arguments: RouterTransitionParameter("/library", "/library"),
      ),
    );
  }

  /// Removes [entry] wherever it is; a route pushed while it was closing may
  /// already be on top of it.
  void remove(BaseRouteEntry entry) {
    history.remove(entry);
  }

  (bool, BaseRouteEntry?) pop() {
    if (history.isEmpty) return (false, null);

    final routeEntry = history.removeLast();
    return (true, routeEntry);
  }

  /// Takes off the pages above the topmost page [name] and returns that page.
  /// Context menus among them are closed and taken off with them. Returns null
  /// and leaves the history as it is when no page [name] is below the current
  /// one, or a dialog is open above it.
  RouteEntry? popTo(String name) {
    final index = history.lastIndexWhere(
      (entry) => entry is RouteEntry && entry.name == name,
    );
    if (index < 0) return null;
    final above = history.sublist(index + 1);
    if (!above.any((entry) => entry is RouteEntry)) return null;
    if (above.any((entry) => entry is ModalRouteEntry)) return null;
    history.removeRange(index + 1, history.length);
    for (final menu in above.reversed.whereType<ContextMenuRouteEntry>()) {
      menu.pop();
    }
    return history[index] as RouteEntry;
  }

  void replace(RouteSettings settings) {
    if (history.isNotEmpty) {
      history.removeLast();
    }
    history.add(RouteEntry.fromSettings(settings));
  }

  /// Whether the page under the current one is [name], with nothing open over
  /// the current page. Dialogs and context menus left under the current page
  /// are looked past, as [previousPage] looks past them.
  bool previousIs(String name) =>
      history.lastOrNull is RouteEntry && previousPage?.name == name;

  /// Like [previousIs], but ignoring dialogs and context menus open over the
  /// current route, for what the header shows rather than what a tap does.
  bool previousPageIs(String name) => previousPage?.name == name;

  /// The page under the current one, ignoring dialogs and context menus
  /// wherever they are, over the current page or left between pages, as
  /// [previousPageIs] looks at it.
  RouteEntry? get previousPage {
    final pages = history.whereType<RouteEntry>().toList();
    return pages.length < 2 ? null : pages[pages.length - 2];
  }

  /// The page shown, under any dialogs and context menus open over it.
  RouteEntry? get currentPage => history.whereType<RouteEntry>().lastOrNull;

  BaseRouteEntry? get current => history.isNotEmpty ? history.last : null;
  bool get isCurrentModal => current is ModalRouteEntry;
  bool get isCurrentContextMenu => current is ContextMenuRouteEntry;
}
