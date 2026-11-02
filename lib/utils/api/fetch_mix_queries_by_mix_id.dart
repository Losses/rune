import '../../bindings/bindings.dart';

class UnavailableMixSelection implements Exception {
  const UnavailableMixSelection();

  @override
  String toString() =>
      'Some selections in this mix are not available on this device.';
}

List<(String, String)> executableMixQueries(List<StoredMixQuery> queries) {
  if (queries.any((query) => !query.available)) {
    throw const UnavailableMixSelection();
  }
  return queries.map((query) => (query.operator, query.parameter)).toList();
}

Future<List<StoredMixQuery>> fetchStoredMixQueriesByMixId(int mixId) async {
  final response = FetchMixQueriesResponse.rustSignalStream.first;
  FetchMixQueriesRequest(mixId: mixId).sendSignalToRust();
  return (await response).message.result;
}

Future<List<(String, String)>> fetchMixQueriesByMixId(int mixId) async =>
    executableMixQueries(await fetchStoredMixQueriesByMixId(mixId));
