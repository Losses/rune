import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<void> removeItemFromPlaylist(int playlistId, String occurrenceId) async {
  final rustSignal = await awaitResponse(
    'RemoveItemFromPlaylistRequest',
    RemoveItemFromPlaylistResponse.rustSignalStream,
    send: () => RemoveItemFromPlaylistRequest(
      playlistId: playlistId,
      occurrenceId: occurrenceId,
    ).sendSignalToRust(),
  );
  final response = rustSignal.message;

  if (!response.success) {
    throw response.error;
  }
}
