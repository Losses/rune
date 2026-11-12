import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<Playlist> getPlaylistById(int playlistId) async {
  final fetchMediaFiles = GetPlaylistByIdRequest(playlistId: playlistId);

  // Listen for the response from Rust
  final rustSignal = await awaitResponse(
    'GetPlaylistByIdRequest',
    GetPlaylistByIdResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) =>
        fetchMediaFiles.copyWith(callId: () => callId).sendSignalToRust(),
  );
  final playlist = rustSignal.message.playlist;

  return playlist;
}
