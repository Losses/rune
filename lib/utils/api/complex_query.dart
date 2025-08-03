import '../../bindings/bindings.dart';

Future<ComplexQueryResponse?> complexQuery(List<ComplexQuery> queries) async {
  if (queries.isEmpty) return null;

  final fetchLibrarySummary = ComplexQueryRequest(queries: queries);
  fetchLibrarySummary.sendSignalToRust();

  final rustSignal = await ComplexQueryResponse.rustSignalStream.first;
  final result = rustSignal.message;

  return result;
}
