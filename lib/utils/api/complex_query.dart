import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<ComplexQueryResponse?> complexQuery(List<ComplexQuery> queries) async {
  if (queries.isEmpty) return null;

  final fetchLibrarySummary = ComplexQueryRequest(queries: queries);

  final rustSignal = await awaitResponse(
    'ComplexQueryRequest',
    ComplexQueryResponse.rustSignalStream,
    send: () => fetchLibrarySummary.sendSignalToRust(),
  );
  final result = rustSignal.message;

  return result;
}
