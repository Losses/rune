import 'package:player/messages/playlist.pbserver.dart';

Future<void> addItemToPlaylist(int playlistId, int itemId,
    [int? position]) async {
  final request = AddItemToPlaylistRequest(
    playlistId: playlistId,
    mediaFileId: playlistId,
    position: position,
  );
  request.sendSignalToRust(); // GENERATED

  await AddItemToPlaylistResponse.rustSignalStream.first;
}
