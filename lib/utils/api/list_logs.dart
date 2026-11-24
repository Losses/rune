import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<List<LogDetail>> listLogs(int cursor, int pageSize) async {
  final listRequest = ListLogRequest(cursor: cursor, pageSize: pageSize);
  final rustSignal = await awaitResponse(
    'ListLogRequest',
    ListLogResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) =>
        listRequest.copyWith(callId: () => callId).sendSignalToRust(),
  );
  final response = rustSignal.message;

  return response.result;
}
