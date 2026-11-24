import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<void> removePlaylist(int playlistId) async {
  final updateRequest = RemovePlaylistRequest(playlistId: playlistId);

  await awaitResponse(
    'RemovePlaylistRequest',
    RemovePlaylistResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) =>
        updateRequest.copyWith(callId: () => callId).sendSignalToRust(),
  );
}
