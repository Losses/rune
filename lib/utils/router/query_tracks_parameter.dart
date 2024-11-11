import 'router_transition_parameter.dart';

class QueryTracksParameter {
  int id;
  String title;

  QueryTracksParameter(this.id, this.title);
}

QueryTracksParameter? getQueryTracksParameter() {
  final parameters = getRouterTransitionParameter()?.parameters;

  if (parameters is! QueryTracksParameter) {
    return null;
  }

  return parameters;
}
