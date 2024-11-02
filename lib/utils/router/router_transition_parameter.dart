class RouterTransitionParameter {
  String from;
  String to;
  Object? parameters;

  RouterTransitionParameter(this.from, this.to, [this.parameters]);
}

RouterTransitionParameter? getRouterTransitionParameter(Object? x) {
  if (x is! RouterTransitionParameter) return null;

  return x;
}
