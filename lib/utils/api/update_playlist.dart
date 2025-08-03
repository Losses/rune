import '../../bindings/bindings.dart';

Future<Playlist> updatePlaylist(
  int playlistId,
  String name,
  String group,
) async {
  final updateRequest = UpdatePlaylistRequest(
    playlistId: playlistId,
    name: name,
    group: group,
  );
  updateRequest.sendSignalToRust(); // GENERATED

  // Listen for the response from Rust
  final rustSignal = await UpdatePlaylistResponse.rustSignalStream.first;
  final response = rustSignal.message;

  return response.playlist;
}
