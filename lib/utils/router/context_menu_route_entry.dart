import '../../utils/router/base_route_entry.dart';

class ContextMenuRouteEntry extends BaseRouteEntry {
  final (bool, dynamic) Function()? canPop;
  final void Function() pop;

  ContextMenuRouteEntry({
    required super.name,
    super.arguments,
    this.canPop,
    required this.pop,
  });
}
