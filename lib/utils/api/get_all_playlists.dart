import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<List<Playlist>> getAllPlaylists() async {
  final fetchRequest = FetchAllPlaylistsRequest();

  // Listen for the response from Rust
  final rustSignal = await awaitResponse(
    'FetchAllPlaylistsRequest',
    FetchAllPlaylistsResponse.rustSignalStream,
    send: () => fetchRequest.sendSignalToRust(),
  );
  final response = rustSignal.message;

  return response.playlists;
}
