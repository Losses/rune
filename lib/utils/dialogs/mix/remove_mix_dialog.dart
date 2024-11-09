import 'package:fluent_ui/fluent_ui.dart';

import '../../../widgets/responsive_dialog_actions.dart';

import '../../api/remove_mix.dart';
import '../../router/navigation.dart';

import '../remove_dialog_on_band.dart';

Future<bool?> showRemoveMixDialog(BuildContext context, int mixId) async {
  final result = await $showModal<bool>(
    context,
    (context, $close) => RemoveMixDialog(
      mixId: mixId,
      $close: $close,
    ),
    dismissWithEsc: true,
    barrierDismissible: true,
  );

  return result;
}

class RemoveMixDialog extends StatefulWidget {
  final int mixId;
  final void Function(bool?) $close;

  const RemoveMixDialog({
    super.key,
    required this.mixId,
    required this.$close,
  });

  @override
  State<RemoveMixDialog> createState() => _RemoveMixDialogState();
}

class _RemoveMixDialogState extends State<RemoveMixDialog> {
  bool isLoading = false;

  void _onConfirm() async {
    setState(() {
      isLoading = true;
    });
    await removeMix(widget.mixId);

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
            Text('Remove Mix'),
          ],
        ),
        content: const Text(
          'If you delete this mix, you won\'t be able to recover it. Do you want to delete it?',
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
