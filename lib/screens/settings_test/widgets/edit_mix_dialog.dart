import 'package:fluent_ui/fluent_ui.dart';

import '../../../screens/settings_test/widgets/mix_editor.dart';

class EditMixDialog extends StatefulWidget {
  const EditMixDialog({super.key});

  @override
  State<EditMixDialog> createState() => _EditMixDialogState();
}

class _EditMixDialogState extends State<EditMixDialog> {
  @override
  Widget build(BuildContext context) {
    final height = MediaQuery.of(context).size.height;
    const reduce = 400.0;

    return ContentDialog(
      title: const Column(
        children: [
          SizedBox(height: 8),
          Text("Create Mix"),
        ],
      ),
      content: Container(
        constraints: BoxConstraints(
          maxHeight: height < reduce ? reduce : height - reduce,
        ),
        child: const MixEditor(),
      ),
      actions: [
        FilledButton(
          child: const Text('Query'),
          onPressed: () {
            Navigator.pop(context, 'User deleted file');
            // Delete file here
          },
        ),
        Button(
          child: const Text('Cancel'),
          onPressed: () => Navigator.pop(context, 'User canceled dialog'),
        ),
      ],
    );
  }
}
