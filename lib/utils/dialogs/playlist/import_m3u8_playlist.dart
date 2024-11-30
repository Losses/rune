import 'package:file_selector/file_selector.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/router/navigation.dart';
import '../../../utils/dialogs/playlist/create_edit_playlist_dialog.dart';

import '../../../messages/playlist.pb.dart';

Future<Playlist?> showCreateImportM3u8PlaylistDialog(
  BuildContext context) async {
  const XTypeGroup typeGroup = XTypeGroup(
    label: 'playlist',
    extensions: <String>['m3u', 'm3u8'],
  );
  final XFile? file = await openFile(
    acceptedTypeGroups: <XTypeGroup>[typeGroup],
  );

  if (file == null) return null;
  if (!context.mounted) return null;

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
