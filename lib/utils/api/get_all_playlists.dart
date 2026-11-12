import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<List<Playlist>> getAllPlaylists() async {
  final fetchRequest = FetchAllPlaylistsRequest();

  // Listen for the response from Rust
  final rustSignal = await awaitResponse(
    'FetchAllPlaylistsRequest',
    FetchAllPlaylistsResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) =>
        fetchRequest.copyWith(callId: () => callId).sendSignalToRust(),
  );
  final response = rustSignal.message;

  return response.playlists;
}
