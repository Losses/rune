import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<Playlist> createPlaylist(String name, String group) async {
  final createRequest = CreatePlaylistRequest(
    name: name,
    group: group.isEmpty ? 'Favorite' : group,
  );

  // Listen for the response from Rust
  final rustSignal = await awaitResponse(
    'CreatePlaylistRequest',
    CreatePlaylistResponse.rustSignalStream,
    send: () => createRequest.sendSignalToRust(),
  );
  final response = rustSignal.message;

  return response.playlist;
}
