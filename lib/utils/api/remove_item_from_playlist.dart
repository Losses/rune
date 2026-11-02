import '../../bindings/bindings.dart';

Future<void> removeItemFromPlaylist(int playlistId, String occurrenceId) async {
  RemoveItemFromPlaylistRequest(
    playlistId: playlistId,
    occurrenceId: occurrenceId,
  ).sendSignalToRust();

  final rustSignal =
      await RemoveItemFromPlaylistResponse.rustSignalStream.first;
  final response = rustSignal.message;

  if (!response.success) {
    throw response.error;
  }
}
