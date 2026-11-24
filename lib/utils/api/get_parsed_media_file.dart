import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<FetchParsedMediaFileResponse> getParsedMediaFile(int fileId) async {
  final fetchRequest = FetchParsedMediaFileRequest(id: fileId);

  final rustSignal = await awaitResponse(
    'FetchParsedMediaFileRequest',
    FetchParsedMediaFileResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) =>
        fetchRequest.copyWith(callId: () => callId).sendSignalToRust(),
  );
  final response = rustSignal.message;

  return response;
}
