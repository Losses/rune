import 'package:fluent_ui/fluent_ui.dart';
import 'package:fast_file_picker/fast_file_picker.dart';

import '../../../bindings/bindings.dart';
import '../../l10n.dart';
import '../information/error.dart';
import 'create_edit_playlist.dart';

Future<Playlist?> showCreateImportM3u8PlaylistDialog(
  BuildContext context,
  FastFilePickerPath file,
) async {
  final path = file.path ?? file.uri;
  if (path == null || path.isEmpty) {
    final s = S.of(context);
    await showErrorDialog(
      context: context,
      title: s.importM3u8,
      errorMessage: s.requestUnavailable,
    );
    return null;
  }
  return showCreateEditPlaylistDialog(context, file.name, m3u8Path: path);
}
