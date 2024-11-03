import 'package:fluent_ui/fluent_ui.dart';

import '../../providers/router_path.dart';
import '../../utils/router/router_transition_parameter.dart';
import '../../widgets/router/rune_with_navigation_bar_and_playback_controllor.dart';

class NavigationHistory {
  final List<RouteSettings> _history = [];

  void push(RouteSettings settings) {
    _history.add(settings);
  }

  void pop() {
    if (_history.isNotEmpty) {
      _history.removeLast();
    }
  }

  void replace(RouteSettings settings) {
    if (_history.isNotEmpty) {
      _history.removeLast();
    }
    _history.add(settings);
  }

  RouteSettings? get current => _history.isNotEmpty ? _history.last : null;
}

final NavigationHistory _history = NavigationHistory();

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
  return _history._history.length > 1;
}

bool $pop() {
  final navigation = $state();

  if (navigation.canPop()) {
    navigation.pop();
    _history.pop();

    final route = _history._history.last;
    $router.update(route.name, route.arguments);

    return true;
  }

  return false;
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
  _history.push(newSettings);

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
  _history.replace(newSettings);

  return navigation.pushReplacementNamed(routeName, arguments: p);
}
