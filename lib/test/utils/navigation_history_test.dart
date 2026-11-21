import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../utils/router/modal_route_entry.dart';
import '../../utils/router/navigation_history.dart';

void main() {
  test('the page a route was opened from is the one under it', () {
    final history = NavigationHistory()
      ..push(const RouteSettings(name: '/albums'))
      ..push(const RouteSettings(name: '/albums/detail'));

    expect(history.previousIs('/albums'), isTrue);
    expect(history.previousIs('/library'), isFalse);
  });

  test('a dialog over the route hides the page under it', () {
    final history = NavigationHistory()
      ..push(const RouteSettings(name: '/albums'))
      ..push(const RouteSettings(name: '/albums/detail'))
      ..pushModal(ModalRouteEntry(name: 'modal', pop: () {}));

    expect(history.previousIs('/albums'), isFalse);
    expect(history.previousIs('/albums/detail'), isFalse);
  });
}
