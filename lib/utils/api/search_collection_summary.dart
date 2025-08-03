import '../../bindings/bindings.dart';

Future<List<(int, String)>> fetchCollectionSummary(
    CollectionType collectionType) async {
  SearchCollectionSummaryRequest(n: 50).sendSignalToRust();

  return (await SearchCollectionSummaryResponse.rustSignalStream.first)
      .message
      .result
      .map((x) => (x.id, x.name))
      .toList();
}
