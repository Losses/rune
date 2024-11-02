import 'router_transition_parameter.dart';

class QueryTracksParameter {
  int id;
  String title;

  QueryTracksParameter(this.id, this.title);
}

QueryTracksParameter? getQueryTracksParameter(Object? x) {
  final parameters = getRouterTransitionParameter(x)?.parameters;

  if (parameters is! QueryTracksParameter) {
    return null;
  }

  return parameters;
}
