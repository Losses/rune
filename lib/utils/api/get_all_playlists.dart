import 'package:player/messages/playlist.pbserver.dart';

Future<List<PlaylistWithoutCoverIds>> getAllPlaylists() async {
  final fetchRequest = FetchAllPlaylistsRequest();
  fetchRequest.sendSignalToRust(); // GENERATED

  // Listen for the response from Rust
  final rustSignal = await FetchAllPlaylistsResponse.rustSignalStream.first;
  final response = rustSignal.message;

  return response.playlists;
}
