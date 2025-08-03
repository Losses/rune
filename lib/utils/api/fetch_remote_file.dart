import '../../bindings/bindings.dart';

Future<String> fetchRemoteFile(String url) async {
  FetchRemoteFileRequest(url: url).sendSignalToRust();

  final rustSignal = await FetchRemoteFileResponse.rustSignalStream.first;
  final response = rustSignal.message;

  if (!response.success) {
    throw response.error;
  }

  return response.localPath;
}
