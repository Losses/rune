import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../utils/router/context_menu_route_entry.dart';
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

  List<String> names(NavigationHistory history) => [
    for (final entry in history.history) entry.name,
  ];

  test('popping to a page takes off every page above the topmost one', () {
    final history = NavigationHistory()
      ..push(const RouteSettings(name: '/search'))
      ..push(const RouteSettings(name: '/artists/detail'))
      ..push(const RouteSettings(name: '/search'))
      ..push(const RouteSettings(name: '/artists/detail'))
      ..push(const RouteSettings(name: '/albums/detail'));

    final revealed = history.popTo('/search');

    expect(revealed, same(history.history.last));
    expect(names(history), [
      '/library',
      '/search',
      '/artists/detail',
      '/search',
    ]);
  });

  test('popping to a page not below the current one changes nothing', () {
    final history = NavigationHistory()
      ..push(const RouteSettings(name: '/albums'))
      ..push(const RouteSettings(name: '/search'));

    expect(history.popTo('/artists'), isNull);
    expect(history.popTo('/search'), isNull);
    expect(names(history), ['/library', '/albums', '/search']);
  });

  test('the page under the current one is found past menus and dialogs '
      'left between the pages', () {
    final history = NavigationHistory()
      ..push(const RouteSettings(name: '/search'))
      ..pushContextMenu(ContextMenuRouteEntry(name: 'flyout', pop: () {}))
      ..pushModal(ModalRouteEntry(name: 'modal', pop: () {}))
      ..push(const RouteSettings(name: '/albums/detail'));

    expect(history.previousPage?.name, '/search');
    expect(history.previousPageIs('/search'), isTrue);
    expect(history.previousIs('/search'), isTrue);

    history.pushContextMenu(ContextMenuRouteEntry(name: 'flyout', pop: () {}));
    expect(history.previousPage?.name, '/search');
    expect(history.previousIs('/search'), isFalse);
  });

  test('popping to a page closes the menus above it and takes them off', () {
    final closed = <String>[];
    final history = NavigationHistory()
      ..push(const RouteSettings(name: '/search'))
      ..pushContextMenu(
        ContextMenuRouteEntry(name: 'flyout', pop: () => closed.add('left')),
      )
      ..push(const RouteSettings(name: '/albums/detail'))
      ..pushContextMenu(
        ContextMenuRouteEntry(name: 'flyout', pop: () => closed.add('open')),
      );

    final revealed = history.popTo('/search');

    expect(revealed, same(history.history.last));
    expect(names(history), ['/library', '/search']);
    expect(closed, ['open', 'left']);
  });

  test('popping to the page under a menu changes nothing', () {
    var closed = false;
    final history = NavigationHistory()
      ..push(const RouteSettings(name: '/search'))
      ..pushContextMenu(
        ContextMenuRouteEntry(name: 'flyout', pop: () => closed = true),
      );

    expect(history.popTo('/search'), isNull);
    expect(names(history), ['/library', '/search', 'flyout']);
    expect(closed, isFalse);
  });

  test('popping to a page under a dialog changes nothing', () {
    final history = NavigationHistory()
      ..push(const RouteSettings(name: '/search'))
      ..push(const RouteSettings(name: '/albums/detail'))
      ..pushModal(ModalRouteEntry(name: 'modal', pop: () {}));

    expect(history.popTo('/search'), isNull);
    expect(names(history), ['/library', '/search', '/albums/detail', 'modal']);
  });
}
