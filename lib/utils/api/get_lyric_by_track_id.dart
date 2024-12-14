import '../../messages/all.dart';

Future<List<LyricContentLine>> getLyricByTrackId(int fileId) async {
  final updateRequest = GetLyricByTrackIdRequest(id: fileId);
  updateRequest.sendSignalToRust(); // GENERATED

  final rustSignal = await GetLyricByTrackIdResponse.rustSignalStream.first;
  final response = rustSignal.message.lines;

  return response;
}
