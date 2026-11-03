import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<List<(int, String)>> fetchCollectionSummary(
  CollectionType collectionType,
) async {
  return (await awaitResponse(
    'SearchCollectionSummaryRequest',
    SearchCollectionSummaryResponse.rustSignalStream,
    send: () => SearchCollectionSummaryRequest(
      collectionType: collectionType,
      n: 50,
    ).sendSignalToRust(),
  )).message.result.map((x) => (x.id, x.name)).toList();
}
