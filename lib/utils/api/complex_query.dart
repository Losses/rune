import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<ComplexQueryResponse?> complexQuery(List<ComplexQuery> queries) async {
  if (queries.isEmpty) return null;

  final fetchLibrarySummary = ComplexQueryRequest(queries: queries);

  final rustSignal = await awaitResponse(
    'ComplexQueryRequest',
    ComplexQueryResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) =>
        fetchLibrarySummary.copyWith(callId: () => callId).sendSignalToRust(),
  );
  final result = rustSignal.message;

  return result;
}
