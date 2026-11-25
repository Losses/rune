import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<String> addItemToPlaylist(
  int playlistId,
  int itemId, [
  int? position,
]) async {
  final request = AddItemToPlaylistRequest(
    playlistId: playlistId,
    mediaFileId: itemId,
    position: position,
  );

  return (await awaitResponse(
    'AddItemToPlaylistRequest',
    AddItemToPlaylistResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) => request.copyWith(callId: () => callId).sendSignalToRust(),
  )).message.occurrenceId;
}
