import '../../bindings/bindings.dart';
import '../build_collection_query.dart';

int _requestSequence = 0;

/// A saved mix keeps its identity so later edits change the offline selection.
(List<MixQuery>, int?) offlineSelectionFor(CollectionType type, int id) {
  if (type == CollectionType.mix) return (<MixQuery>[], id);
  return (
    buildCollectionQuery(
      type,
      id,
    ).map((q) => MixQuery(operator: q.$1, parameter: q.$2)).toList(),
    null,
  );
}

Future<void> setDownloadRule(CollectionType type, int id, bool enabled) async {
  final (queries, mixId) = offlineSelectionFor(type, id);
  final requestId =
      '${DateTime.now().microsecondsSinceEpoch}-${_requestSequence++}';
  final response = SetDownloadRuleResponse.rustSignalStream
      .firstWhere((event) => event.message.requestId == requestId)
      .timeout(const Duration(seconds: 15));
  SetDownloadRuleRequest(
    requestId: requestId,
    queries: queries,
    mixId: mixId,
    enabled: enabled,
  ).sendSignalToRust();
  final error = (await response).message.error;
  if (error != null) throw StateError(error);
}
