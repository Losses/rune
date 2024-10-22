import 'package:fluent_ui/fluent_ui.dart';
import 'package:rune/widgets/responsive_dialog_actions.dart';

import '../../api/remove_mix.dart';

import '../remove_dialog_on_band.dart';

Future<bool?> showRemoveMixDialog(BuildContext context, int mixId) async {
  final result = await showDialog<bool>(
    context: context,
    builder: (context) => RemoveMixDialog(mixId: mixId),
  );

  return result;
}

class RemoveMixDialog extends StatefulWidget {
  final int mixId;

  const RemoveMixDialog({
    super.key,
    required this.mixId,
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

    Navigator.pop(context, true);
  }

  @override
  Widget build(BuildContext context) {
    return RemoveDialogOnBand(
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
              onPressed: isLoading ? null : () => Navigator.pop(context, false),
              child: const Text('Cancel'),
            ),
          ),
        ],
      ),
    );
  }
}
