import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<FetchParsedMediaFileResponse> getParsedMediaFile(int fileId) async {
  final fetchRequest = FetchParsedMediaFileRequest(id: fileId);

  final rustSignal = await awaitResponse(
    'FetchParsedMediaFileRequest',
    FetchParsedMediaFileResponse.rustSignalStream,
    send: () => fetchRequest.sendSignalToRust(),
  );
  final response = rustSignal.message;

  return response;
}
