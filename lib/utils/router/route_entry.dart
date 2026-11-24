import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/router/base_route_entry.dart';

class RouteEntry extends BaseRouteEntry {
  RouteEntry({
    required super.name,
    super.arguments,
  });

  factory RouteEntry.fromSettings(RouteSettings settings) {
    return RouteEntry(
      name: settings.name ?? '/',
      arguments: settings.arguments,
    );
  }

  RouteSettings toSettings() {
    return RouteSettings(name: name, arguments: arguments);
  }

  @override
  toString() {
    return 'RouteSettings #$name ($arguments)';
  }
}
