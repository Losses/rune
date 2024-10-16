import 'package:fluent_ui/fluent_ui.dart';

import '../../api/remove_playlist.dart';

import '../unavailable_dialog_on_band.dart';

Future<bool?> showRemovePlaylistDialog(
    BuildContext context, int playlistId) async {
  final result = await showDialog<bool>(
    context: context,
    builder: (context) => RemovePlaylistDialog(playlistId: playlistId),
  );

  return result;
}

class RemovePlaylistDialog extends StatefulWidget {
  final int playlistId;

  const RemovePlaylistDialog({
    super.key,
    required this.playlistId,
  });

  @override
  State<RemovePlaylistDialog> createState() => _RemovePlaylistDialogState();
}

class _RemovePlaylistDialogState extends State<RemovePlaylistDialog> {
  bool isLoading = false;

  @override
  Widget build(BuildContext context) {
    return UnavailableDialogOnBand(
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
          Button(
            onPressed: isLoading
                ? null
                : () async {
                    setState(() {
                      isLoading = true;
                    });
                    await removePlaylist(widget.playlistId);

                    if (!context.mounted) return;

                    Navigator.pop(context, true);
                  },
            child: const Text('Delete'),
          ),
          FilledButton(
            onPressed: isLoading ? null : () => Navigator.pop(context, false),
            child: const Text('Cancel'),
          ),
        ],
      ),
    );
  }
}
