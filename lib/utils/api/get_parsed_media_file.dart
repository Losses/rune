import '../../bindings/bindings.dart';

Future<FetchParsedMediaFileResponse> getParsedMediaFile(int fileId) async {
  final fetchRequest = FetchParsedMediaFileRequest(id: fileId);
  fetchRequest.sendSignalToRust(); // GENERATED

  final rustSignal = await FetchParsedMediaFileResponse.rustSignalStream.first;
  final response = rustSignal.message;

  return response;
}
