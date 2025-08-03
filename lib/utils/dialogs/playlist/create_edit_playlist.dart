import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/router/navigation.dart';
import '../../../utils/dialogs/playlist/create_edit_playlist_dialog.dart';

import '../../../bindings/bindings.dart';

Future<Playlist?> showCreateEditPlaylistDialog(
  BuildContext context,
  String? defaultTitle, {
  int? playlistId,
}) async {
  return await $showModal<Playlist?>(
    context,
    (context, $close) => CreateEditPlaylistDialog(
      playlistId: playlistId,
      defaultTitle: defaultTitle,
      $close: $close,
    ),
    dismissWithEsc: true,
    barrierDismissible: true,
  );
}
