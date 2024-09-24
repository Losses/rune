import './query_list.dart';

bool queriesHasRecommendation(QueryList queries) {
  final hasRecommendation =
      queries.toQueryList().indexWhere((x) => x.operator == 'pipe::recommend');

  return hasRecommendation > 0;
}
