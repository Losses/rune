import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<bool> removePlaylist(int playlistId) async {
  final updateRequest = RemovePlaylistRequest(playlistId: playlistId);

  final rustSignal = await awaitResponse(
    'RemovePlaylistRequest',
    RemovePlaylistResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) =>
        updateRequest.copyWith(callId: () => callId).sendSignalToRust(),
  );
  final response = rustSignal.message;

  return response.success;
}
