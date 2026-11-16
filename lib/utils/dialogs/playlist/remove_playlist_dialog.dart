import '../../api/request_failure.dart';
import '../../../bindings/bindings.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/router/navigation.dart';
import '../../../widgets/responsive_dialog_actions.dart';
import '../../../utils/l10n.dart';

import '../../api/remove_playlist.dart';

import '../remove_dialog_on_band.dart';

Future<bool?> showRemovePlaylistDialog(
  BuildContext context,
  int playlistId,
) async {
  final result = await $showModal<bool>(
    staysUsableWhenShrunk: false,
    context,
    (context, $close) =>
        RemovePlaylistDialog(playlistId: playlistId, $close: $close),
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
  RequestFailure? failure;

  void _onConfirm() async {
    setState(() {
      isLoading = true;
    });
    try {
      await removePlaylist(widget.playlistId);
    } on RequestFailure catch (error) {
      if (error.kind != RequestErrorKind.notFound) {
        if (mounted) {
          setState(() {
            isLoading = false;
            failure = error;
          });
        }
        return;
      }
    }

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
        content: Text(
          failure == null
              ? S.of(context).removePlaylistSubtitle
              : requestFailureMessage(S.of(context), failure!),
        ),
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
