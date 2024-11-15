import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/router/navigation.dart';
import '../../../widgets/responsive_dialog_actions.dart';
import '../../../generated/l10n.dart';

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
        title: Column(
          children: [
            const SizedBox(height: 8),
            Text(S.of(context).removePlaylistTitle),
          ],
        ),
        content: Text(S.of(context).removePlaylistSubtitle),
        actions: [
          ResponsiveDialogActions(
            Button(
              onPressed: isLoading ? null : _onConfirm,
              child: Text(S.of(context).delete),
            ),
            FilledButton(
              onPressed: isLoading ? null : () => widget.$close(false),
              child: Text(S.of(context).cancel),
            ),
          ),
        ],
      ),
    );
  }
}
