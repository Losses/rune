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
  request.sendSignalToRust(); // GENERATED

  final response =
      (await AddItemToPlaylistResponse.rustSignalStream.first).message;
  if (!response.success) throw StateError('Playlist insertion failed');
  return response.occurrenceId;
}
