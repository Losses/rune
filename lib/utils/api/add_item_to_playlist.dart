import '../../bindings/bindings.dart';

Future<void> addItemToPlaylist(int playlistId, int itemId,
    [int? position]) async {
  final request = AddItemToPlaylistRequest(
    playlistId: playlistId,
    mediaFileId: itemId,
    position: position,
  );
  request.sendSignalToRust(); // GENERATED

  await AddItemToPlaylistResponse.rustSignalStream.first;
}
