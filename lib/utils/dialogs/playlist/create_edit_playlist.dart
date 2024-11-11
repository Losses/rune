import 'package:fluent_ui/fluent_ui.dart';
import 'package:rune/utils/dialogs/playlist/create_edit_playlist_dialog.dart';
import 'package:rune/utils/router/navigation.dart';

import '../../../messages/playlist.pb.dart';

Future<Playlist?> showCreateEditPlaylistDialog(
  BuildContext context, {
  int? playlistId,
}) async {
  return await $showModal<Playlist?>(
    context,
    (context, $close) => CreateEditPlaylistDialog(
      playlistId: playlistId,
      $close: $close,
    ),
    dismissWithEsc: true,
    barrierDismissible: true,
  );
}
