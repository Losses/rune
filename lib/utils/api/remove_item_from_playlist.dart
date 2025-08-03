import '../../bindings/bindings.dart';

Future<void> removeItemFromPlaylist(
  int playlistId,
  int mediaFileId,
  int position,
) async {
  RemoveItemFromPlaylistRequest(
    playlistId: playlistId,
    mediaFileId: mediaFileId,
    position: position,
  ).sendSignalToRust();

  final rustSignal =
      await RemoveItemFromPlaylistResponse.rustSignalStream.first;
  final response = rustSignal.message;

  if (!response.success) {
    throw response.error;
  }
}
