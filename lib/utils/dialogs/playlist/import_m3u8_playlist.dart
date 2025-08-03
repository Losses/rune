import 'package:file_selector/file_selector.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/router/navigation.dart';
import '../../../utils/dialogs/playlist/create_edit_playlist_dialog.dart';

import '../../../bindings/bindings.dart';

Future<Playlist?> showCreateImportM3u8PlaylistDialog(
  BuildContext context,
  XFile file,
) async {
  return await $showModal<Playlist?>(
    context,
    (context, $close) => CreateEditPlaylistDialog(
      m3u8Path: file.path,
      defaultTitle: file.name,
      $close: $close,
    ),
    dismissWithEsc: true,
    barrierDismissible: true,
  );
}
