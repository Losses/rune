import 'package:fluent_ui/fluent_ui.dart';

import '../../../messages/mix.pb.dart';

import './utils.dart';

class CreateEditMixDialog extends StatefulWidget {
  final int? mixId;
  final (String, String)? operator;

  const CreateEditMixDialog({super.key, this.mixId, this.operator});

  @override
  CreateEditMixDialogState createState() => CreateEditMixDialogState();
}

class CreateEditMixDialogState extends State<CreateEditMixDialog> {
  final titleController = TextEditingController();
  bool isLoading = false;
  List<String> groupList = ['Favorite'];
  String selectedGroup = 'Favorite';

  MixWithoutCoverIds? mix;

  @override
  void initState() {
    super.initState();
    fetchGroupList();
    if (widget.mixId != null) {
      loadMix(widget.mixId!);
    }
  }

  Future<void> fetchGroupList() async {
    final groups = await getGroupList();
    setState(() {
      groupList = ['Favorite', ...groups];
    });
  }

  Future<void> loadMix(int mixId) async {
    mix = await getMixById(mixId);
    if (mix != null) {
      titleController.text = mix!.name;
      selectedGroup = mix!.group;
    }
    setState(() {});
  }

  @override
  Widget build(BuildContext context) {
    return ContentDialog(
      title: Column(
        children: [
          const SizedBox(height: 16),
          Text(widget.mixId != null ? 'Edit Mix' : 'Create Mix'),
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
              enabled: !isLoading,
            ),
          ),
          const SizedBox(height: 16),
          InfoLabel(
            label: 'Group',
            child: EditableComboBox<String>(
              value: selectedGroup,
              items: groupList.map<ComboBoxItem<String>>((e) {
                return ComboBoxItem<String>(
                  value: e,
                  child: Text(e),
                );
              }).toList(),
              onChanged: isLoading
                  ? null
                  : (group) {
                      setState(() => selectedGroup = group ?? selectedGroup);
                    },
              placeholder: const Text('Select a group'),
              onFieldSubmitted: (String text) {
                setState(() => selectedGroup = text);
                return text;
              },
            ),
          ),
          const SizedBox(height: 8),
        ],
      ),
      actions: [
        FilledButton(
          onPressed: isLoading
              ? null
              : () async {
                  setState(() {
                    isLoading = true;
                  });

                  final operator = widget.operator;

                  MixWithoutCoverIds? response;
                  if (widget.mixId != null) {
                    response = await updateMix(
                      widget.mixId!,
                      titleController.text,
                      selectedGroup,
                      false,
                      99,
                      operator == null ? [] : [operator],
                    );
                  } else {
                    response = await createMix(
                      titleController.text,
                      selectedGroup,
                      false,
                      99,
                      operator == null ? [] : [operator],
                    );
                  }

                  setState(() {
                    isLoading = false;
                  });

                  if (!context.mounted) return;
                  Navigator.pop(context, response);
                },
          child: Text(widget.mixId != null ? 'Save' : 'Create'),
        ),
        Button(
          onPressed: isLoading ? null : () => Navigator.pop(context, null),
          child: const Text('Cancel'),
        ),
      ],
    );
  }
}
