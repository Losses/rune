import 'package:fluent_ui/fluent_ui.dart';

void showCreateEditPlaylistDialog(BuildContext context,
    {required bool isEdit}) async {
  final titleController = TextEditingController();
  final groupController = TextEditingController();

  await showDialog<String>(
    context: context,
    builder: (context) => ContentDialog(
      title: Column(
        children: [
          const SizedBox(height: 16),
          Text(isEdit ? 'Edit Playlist' : 'Create Playlist'),
        ],
      ),
      content: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          const SizedBox(height: 16),
          InfoLabel(
            label: 'Title',
            child: TextBox(
              controller: titleController,
            ),
          ),
          const SizedBox(height: 16),
          InfoLabel(
            label: 'Group',
            child: TextBox(
              controller: groupController,
            ),
          ),
          const SizedBox(height: 8),
        ],
      ),
      actions: [
        FilledButton(
          child: Text(isEdit ? 'Save' : 'Create'),
          onPressed: () {
            // Handle save or create action here
            // For example, you can use titleController.text and groupController.text
            Navigator.pop(
                context, 'User ${isEdit ? 'edited' : 'created'} playlist');
          },
        ),
        Button(
          child: const Text('Cancel'),
          onPressed: () => Navigator.pop(context, 'User canceled dialog'),
        ),
      ],
    ),
  );
}
