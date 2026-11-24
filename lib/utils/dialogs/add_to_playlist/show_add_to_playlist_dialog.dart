import 'package:fluent_ui/fluent_ui.dart';

import '../../../bindings/bindings.dart';
import '../../router/navigation.dart';

import 'add_to_playlist_dialog.dart';

/// Adds the songs of the collection [type] [id] to a playlist in a dialog that
/// shows the progress, and completes when the dialog closes.
Future<void> showAddToPlaylistDialog(
  BuildContext context,
  CollectionType type,
  int id,
  int playlistId, {
  required Future<List<int>> Function(CollectionType type, int id) loadFileIds,
  required Future<void> Function(int playlistId, int fileId) addFileToPlaylist,
}) async {
  await $showModal<void>(
    staysUsableWhenShrunk: false,
    context,
    (context, $close) => AddToPlaylistDialog(
      type: type,
      id: id,
      playlistId: playlistId,
      loadFileIds: loadFileIds,
      addFileToPlaylist: addFileToPlaylist,
      $close: $close,
    ),
    dismissWithEsc: true,
    barrierDismissible: true,
  );
}
