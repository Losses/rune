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

  final response = (await awaitResponse(
    'AddItemToPlaylistRequest',
    AddItemToPlaylistResponse.rustSignalStream,
    send: () => request.sendSignalToRust(),
  )).message;
  if (!response.success) throw StateError('Playlist insertion failed');
  return response.occurrenceId;
}
