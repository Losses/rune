import 'request_failure.dart';
import 'query_mix_tracks.dart';
import '../../bindings/bindings.dart';

Future<ComplexQueryResponse?> complexQuery(List<ComplexQuery> queries) async {
  if (queries.isEmpty) return null;

  final fetchLibrarySummary = ComplexQueryRequest(
    queries: queries,
    queue: shownPlayerQueue(),
  );

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
