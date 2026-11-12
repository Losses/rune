import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<String> fetchRemoteFile(String url) async {
  final rustSignal = await awaitResponse(
    'FetchRemoteFileRequest',
    FetchRemoteFileResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) =>
        FetchRemoteFileRequest(url: url, callId: callId).sendSignalToRust(),
  );
  final response = rustSignal.message;

  if (!response.success) {
    throw response.error;
  }

  return response.localPath;
}
