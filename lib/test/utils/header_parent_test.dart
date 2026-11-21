import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../utils/navigation/header_parent.dart';
import '../../utils/router/modal_route_entry.dart';
import '../../utils/router/navigation_history.dart';

void main() {
  test('a page opened from search names search above it', () {
    final history = NavigationHistory()
      ..push(const RouteSettings(name: '/search'))
      ..push(const RouteSettings(name: '/artists/detail'));

    expect(headerParent('/artists/detail', history, false)?.path, '/search');
  });

  test('a page opened from its list names that list above it', () {
    final history = NavigationHistory()
      ..push(const RouteSettings(name: '/artists'))
      ..push(const RouteSettings(name: '/artists/detail'));

    expect(headerParent('/artists/detail', history, false)?.path, '/artists');
  });

  test('a dialog over a page opened from search keeps search above it', () {
    final history = NavigationHistory()
      ..push(const RouteSettings(name: '/search'))
      ..push(const RouteSettings(name: '/artists/detail'))
      ..pushModal(ModalRouteEntry(name: 'modal', pop: () {}));

    expect(headerParent('/artists/detail', history, false)?.path, '/search');
  });

  test('a tap on the page already shown does nothing', () {
    final history = NavigationHistory()
      ..push(const RouteSettings(name: '/search'))
      ..push(const RouteSettings(name: '/artists/detail'));

    expect(
      headerTapAction('/search', '/artists/detail', history),
      HeaderTapAction.pop,
    );
    history.pop();
    expect(
      headerTapAction('/search', '/search', history),
      HeaderTapAction.none,
    );
  });

  test('a tap on a parent that is not below the page replaces', () {
    final history =
        NavigationHistory()..push(const RouteSettings(name: '/artists/detail'));

    expect(
      headerTapAction('/artists', '/artists/detail', history),
      HeaderTapAction.replace,
    );
  });
}
