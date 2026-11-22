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
  /// Returns null and leaves the history as it is when no page [name] is
  /// below the current one, or something other than a page is open above it.
  RouteEntry? popTo(String name) {
    final index = history.lastIndexWhere(
      (entry) => entry is RouteEntry && entry.name == name,
    );
    if (index < 0 || index == history.length - 1) return null;
    if (history.skip(index + 1).any((entry) => entry is! RouteEntry)) {
      return null;
    }
    history.removeRange(index + 1, history.length);
    return history[index] as RouteEntry;
  }

  void replace(RouteSettings settings) {
    if (history.isNotEmpty) {
      history.removeLast();
    }
    history.add(RouteEntry.fromSettings(settings));
  }

  /// Whether the route under the current one is [name], with nothing open over
  /// the current route.
  bool previousIs(String name) {
    if (history.length < 2 || history.last is! RouteEntry) return false;
    final previous = history[history.length - 2];
    return previous is RouteEntry && previous.name == name;
  }

  /// Like [previousIs], but ignoring dialogs and context menus open over the
  /// current route, for what the header shows rather than what a tap does.
  bool previousPageIs(String name) {
    final pages = history.whereType<RouteEntry>().toList();
    if (pages.length < 2) return false;
    final index = history.indexOf(pages.last);
    if (index < 1 || history[index - 1] is! RouteEntry) return false;
    return pages[pages.length - 2].name == name;
  }

  BaseRouteEntry? get current => history.isNotEmpty ? history.last : null;
  bool get isCurrentModal => current is ModalRouteEntry;
  bool get isCurrentContextMenu => current is ContextMenuRouteEntry;
}
