import 'package:fluent_ui/fluent_ui.dart';

import '../../api/remove_mix.dart';

void showRemoveMixDialog(BuildContext context, int mixId) async {
  final result = await showDialog<bool>(
    context: context,
    builder: (context) => RemoveMixDialog(mixId: mixId),
  );

  if (result ?? false) {}
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

  @override
  Widget build(BuildContext context) {
    return ContentDialog(
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
        Button(
          onPressed: isLoading
              ? null
              : () async {
                  setState(() {
                    isLoading = true;
                  });
                  await removeMix(widget.mixId);

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
    );
  }
}
