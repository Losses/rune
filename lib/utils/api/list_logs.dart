import '../../bindings/bindings.dart';

Future<List<LogDetail>> listLogs(int cursor, int pageSize) async {
  final listRequest = ListLogRequest(
    cursor: cursor,
    pageSize: pageSize,
  );
  listRequest.sendSignalToRust();

  final rustSignal = await ListLogResponse.rustSignalStream.first;
  final response = rustSignal.message;

  return response.result;
}
