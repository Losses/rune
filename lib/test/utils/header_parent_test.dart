import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../utils/navigation/header_parent.dart';
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
}
