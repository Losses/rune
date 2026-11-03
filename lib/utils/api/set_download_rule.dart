import 'request_failure.dart';
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

Future<void> setDownloadRule(
  CollectionType type,
  int id,
  bool enabled, {
  bool deleteUnselectedFiles = false,
}) async {
  final (queries, mixId) = offlineSelectionFor(type, id);
  final requestId =
      '${DateTime.now().microsecondsSinceEpoch}-${_requestSequence++}';
  final response = awaitResponse(
    'SetDownloadRuleRequest',
    SetDownloadRuleResponse.rustSignalStream.where(
      (event) => event.message.requestId == requestId,
    ),
    send: () => SetDownloadRuleRequest(
      requestId: requestId,
      queries: queries,
      mixId: mixId,
      enabled: enabled,
      deleteUnselectedFiles: deleteUnselectedFiles,
    ).sendSignalToRust(),
    timeout: const Duration(seconds: 15),
  );
  final error = (await response).message.error;
  if (error != null) throw StateError(error);
}
