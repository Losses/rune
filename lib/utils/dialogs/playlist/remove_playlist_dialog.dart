import 'package:fluent_ui/fluent_ui.dart';
import 'package:rune/utils/router/navigation.dart';
import 'package:rune/widgets/responsive_dialog_actions.dart';

import '../../api/remove_playlist.dart';

import '../remove_dialog_on_band.dart';

Future<bool?> showRemovePlaylistDialog(
    BuildContext context, int playlistId) async {
  final result = await $showModal<bool>(
    context,
    (context, $close) => RemovePlaylistDialog(
      playlistId: playlistId,
      $close: $close,
    ),
    dismissWithEsc: true,
    barrierDismissible: true,
  );

  return result;
}

class RemovePlaylistDialog extends StatefulWidget {
  final int playlistId;
  final void Function(bool?) $close;

  const RemovePlaylistDialog({
    super.key,
    required this.playlistId,
    required this.$close,
  });

  @override
  State<RemovePlaylistDialog> createState() => _RemovePlaylistDialogState();
}

class _RemovePlaylistDialogState extends State<RemovePlaylistDialog> {
  bool isLoading = false;

  void _onConfirm() async {
    setState(() {
      isLoading = true;
    });
    await removePlaylist(widget.playlistId);

    if (!mounted) return;

    widget.$close(true);
  }

  @override
  Widget build(BuildContext context) {
    return RemoveDialogOnBand(
      $close: widget.$close,
      onConfirm: _onConfirm,
      child: ContentDialog(
        title: const Column(
          children: [
            SizedBox(height: 8),
            Text('Remove Playlist'),
          ],
        ),
        content: const Text(
          'If you delete this playlist, you won\'t be able to recover it. Do you want to delete it?',
        ),
        actions: [
          ResponsiveDialogActions(
            Button(
              onPressed: isLoading ? null : _onConfirm,
              child: const Text('Delete'),
            ),
            FilledButton(
              onPressed: isLoading ? null : () => widget.$close(false),
              child: const Text('Cancel'),
            ),
          ),
        ],
      ),
    );
  }
}
