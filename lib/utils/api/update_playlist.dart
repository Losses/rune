import 'request_failure.dart';
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

  // Listen for the response from Rust
  final rustSignal = await awaitResponse(
    'UpdatePlaylistRequest',
    UpdatePlaylistResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) =>
        updateRequest.copyWith(callId: () => callId).sendSignalToRust(),
  );
  final response = rustSignal.message;

  return response.playlist;
}
