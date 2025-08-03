import '../../bindings/bindings.dart';

Future<Playlist> createPlaylist(
  String name,
  String group,
) async {
  final createRequest = CreatePlaylistRequest(name: name, group: group.isEmpty ? 'Favorite' : group);
  createRequest.sendSignalToRust(); // GENERATED

  // Listen for the response from Rust
  final rustSignal = await CreatePlaylistResponse.rustSignalStream.first;
  final response = rustSignal.message;

  return response.playlist;
}
