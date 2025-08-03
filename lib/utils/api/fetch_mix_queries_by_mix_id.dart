import '../../bindings/bindings.dart';

Future<List<(String, String)>> fetchMixQueriesByMixId(int mixId) async {
  final fetchMediaFiles = FetchMixQueriesRequest(mixId: mixId);
  fetchMediaFiles.sendSignalToRust(); // GENERATED

  // Listen for the response from Rust
  final rustSignal = await FetchMixQueriesResponse.rustSignalStream.first;
  final queries = rustSignal.message.result;

  return queries.map((x) => (x.operator, x.parameter)).toList();
}
