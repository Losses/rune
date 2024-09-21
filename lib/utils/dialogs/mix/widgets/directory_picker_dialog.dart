import 'package:fluent_ui/fluent_ui.dart';
import 'package:player/widgets/directory/directory_tree.dart';

class DirectoryPickerDialog extends StatelessWidget {
  final DirectoryTreeController controller;
  const DirectoryPickerDialog({super.key, required this.controller});

  @override
  Widget build(BuildContext context) {
    return ContentDialog(
      title: const Column(
        children: [
          SizedBox(height: 8),
          Text("Pick Directory"),
        ],
      ),
      content: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          DirectoryTree(
            controller: controller,
          )
        ],
      ),
      actions: [
        FilledButton(
          child: const Text('Confirm'),
          onPressed: () {
            Navigator.pop(context, controller.value);
            // Delete file here
          },
        ),
        Button(
          child: const Text('Cancel'),
          onPressed: () => Navigator.pop(context, null),
        ),
      ],
    );
  }
}
