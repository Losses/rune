import 'package:fluent_ui/fluent_ui.dart';
import 'package:rune/utils/router/router_transition_parameter.dart';

Future<T?> $push<T extends Object?>(
  BuildContext context,
  String routeName, {
  Object? arguments,
}) {
  final from = ModalRoute.of(context)?.settings.name ?? "/";
  final to = routeName;
  return Navigator.pushNamed(
    context,
    routeName,
    arguments: RouterTransitionParameter(from, to, arguments),
  );
}

Future<T?> $replace<T extends Object?>(
  BuildContext context,
  String routeName, {
  Object? arguments,
}) {
  final from = ModalRoute.of(context)?.settings.name ?? "/";
  final to = routeName;
  return Navigator.pushReplacementNamed(
    context,
    routeName,
    arguments: RouterTransitionParameter(from, to, arguments),
  );
}
