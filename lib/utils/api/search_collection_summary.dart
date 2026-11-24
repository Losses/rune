import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<List<(int, String)>> fetchCollectionSummary(
  CollectionType collectionType,
) async {
  return (await awaitResponse(
    'SearchCollectionSummaryRequest',
    SearchCollectionSummaryResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) => SearchCollectionSummaryRequest(
      collectionType: collectionType,
      n: 50,
      callId: callId,
    ).sendSignalToRust(),
  )).message.result.map((x) => (x.id, x.name)).toList();
}
