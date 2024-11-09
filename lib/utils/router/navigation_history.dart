import 'package:fluent_ui/fluent_ui.dart';

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

  reset() {
    history.clear();
    history.add(
      RouteEntry(
        name: "/library",
        arguments: RouterTransitionParameter("/library", "/library"),
      ),
    );
  }

  (bool, dynamic) pop() {
    if (history.isEmpty) return (false, null);

    history.removeLast();
    return (true, null);
  }

  void replace(RouteSettings settings) {
    if (history.isNotEmpty) {
      history.removeLast();
    }
    history.add(RouteEntry.fromSettings(settings));
  }

  BaseRouteEntry? get current => history.isNotEmpty ? history.last : null;
  bool get isCurrentModal => current is ModalRouteEntry;
}
