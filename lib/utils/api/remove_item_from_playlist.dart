import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<void> removeItemFromPlaylist(int playlistId, String occurrenceId) async {
  await awaitResponse(
    'RemoveItemFromPlaylistRequest',
    RemoveItemFromPlaylistResponse.rustSignalStream,
    send: () => RemoveItemFromPlaylistRequest(
      playlistId: playlistId,
      occurrenceId: occurrenceId,
    ).sendSignalToRust(),
  );
}
