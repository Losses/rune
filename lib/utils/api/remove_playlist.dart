import 'package:player/messages/playlist.pb.dart';

Future<bool> removePlaylist(int playlistId) async {
  final updateRequest = RemovePlaylistRequest(playlistId: playlistId);
  updateRequest.sendSignalToRust(); // GENERATED

  final rustSignal = await RemovePlaylistResponse.rustSignalStream.first;
  final response = rustSignal.message;

  return response.success;
}
