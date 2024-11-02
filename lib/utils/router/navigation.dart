import 'package:fluent_ui/fluent_ui.dart';
import 'package:rune/utils/router/router_path.dart';

import '../../utils/router/constants/navigator.dart';
import '../../utils/router/router_transition_parameter.dart';

Future<T?> $push<T extends Object?>(
  BuildContext context,
  String routeName, {
  Object? arguments,
}) async {
  final from = $routerPath.path;
  final to = routeName;

  final p = RouterTransitionParameter(from, to, arguments);

  $routerPath.update(p);

  return (await $navigator.future).pushNamed(
    routeName,
    arguments: p,
  );
}

Future<T?> $replace<T extends Object?>(
  BuildContext context,
  String routeName, {
  Object? arguments,
}) async {
  final from = $routerPath.path;
  final to = routeName;

  final p = RouterTransitionParameter(from, to, arguments);

  $routerPath.update(p);

  return (await $navigator.future).pushReplacementNamed(
    routeName,
    arguments: p,
  );
}
