import '../../messages/all.dart';

Future<bool> getLiked(int fileId) async {
  final updateRequest = GetLikedRequest(fileId: fileId);
  updateRequest.sendSignalToRust(); // GENERATED

  final rustSignal = await GetLikedResponse.rustSignalStream.first;
  final response = rustSignal.message;

  return response.liked;
}
