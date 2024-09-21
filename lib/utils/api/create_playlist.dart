import 'package:player/messages/playlist.pb.dart';

Future<PlaylistWithoutCoverIds> createPlaylist(
  String name,
  String group,
) async {
  final createRequest = CreatePlaylistRequest(name: name, group: group);
  createRequest.sendSignalToRust(); // GENERATED

  // Listen for the response from Rust
  final rustSignal = await CreatePlaylistResponse.rustSignalStream.first;
  final response = rustSignal.message;

  return response.playlist;
}
