import 'package:rune/messages/stat.pb.dart';

Future<bool?> setLiked(int fileId, bool liked) async {
  final updateRequest = SetLikedRequest(fileId: fileId, liked: liked);
  updateRequest.sendSignalToRust(); // GENERATED

  final rustSignal = await SetLikedResponse.rustSignalStream.first;
  final response = rustSignal.message;

  return response.success ? liked : null;
}
