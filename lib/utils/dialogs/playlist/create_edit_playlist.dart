import 'package:fluent_ui/fluent_ui.dart';
import 'package:player/utils/dialogs/playlist/create_edit_playlist_dialog.dart';

import '../../../messages/playlist.pb.dart';

Future<Playlist?> showCreateEditPlaylistDialog(
    BuildContext context,
    {int? playlistId}) async {
  return await showDialog<Playlist?>(
    context: context,
    builder: (context) => CreateEditPlaylistDialog(playlistId: playlistId),
  );
}
