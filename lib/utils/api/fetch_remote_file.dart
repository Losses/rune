import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<String> fetchRemoteFile(String url) async {
  final rustSignal = await awaitResponse(
    'FetchRemoteFileRequest',
    FetchRemoteFileResponse.rustSignalStream,
    send: () => FetchRemoteFileRequest(url: url).sendSignalToRust(),
  );
  final response = rustSignal.message;

  if (!response.success) {
    throw response.error;
  }

  return response.localPath;
}
