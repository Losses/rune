import '../../utils/router/navigation.dart';

class RouterTransitionParameter {
  String from;
  String to;
  Object? parameters;

  RouterTransitionParameter(this.from, this.to, [this.parameters]);
}

RouterTransitionParameter? getRouterTransitionParameter() {
  final x = $arguments();
  if (x is! RouterTransitionParameter) return null;

  return x;
}
