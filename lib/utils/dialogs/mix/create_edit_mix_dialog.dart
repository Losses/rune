import 'package:fluent_ui/fluent_ui.dart';
import 'package:rune/widgets/responsive_dialog_actions.dart';

import '../../../messages/mix.pb.dart';
import '../../../messages/collection.pb.dart';

import '../../api/create_mix.dart';
import '../../api/update_mix.dart';
import '../../api/get_mix_by_id.dart';
import '../../api/fetch_collection_group_summary_title.dart';

import '../unavailable_dialog_on_band.dart';

class CreateEditMixDialog extends StatefulWidget {
  final int? mixId;
  final (String, String)? operator;

  const CreateEditMixDialog({super.key, this.mixId, this.operator});

  @override
  CreateEditMixDialogState createState() => CreateEditMixDialogState();
}

class CreateEditMixDialogState extends State<CreateEditMixDialog> {
  final titleController = TextEditingController();
  final groupController = TextEditingController();
  bool isLoading = false;
  List<String> groupList = ['Favorite'];

  Mix? mix;

  @override
  void initState() {
    super.initState();
    fetchGroupList();
    if (widget.mixId != null) {
      loadMix(widget.mixId!);
    }
  }

  Future<void> fetchGroupList() async {
    final groups = await fetchCollectionGroupSummaryTitle(CollectionType.Mix);
    setState(() {
      groupList = ['Favorite', ...groups];
    });
  }

  Future<void> loadMix(int mixId) async {
    mix = await getMixById(mixId);
    if (mix != null) {
      titleController.text = mix!.name;
      groupController.text = mix!.group;
    }
    setState(() {});
  }

  @override
  Widget build(BuildContext context) {
    return UnavailableDialogOnBand(
      child: ContentDialog(
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
              child: AutoSuggestBox<String>(
                controller: groupController,
                items: groupList.map<AutoSuggestBoxItem<String>>(
                  (e) {
                    return AutoSuggestBoxItem<String>(
                      value: e,
                      label: e,
                    );
                  },
                ).toList(),
                placeholder: "Select a group",
              ),
            ),
            const SizedBox(height: 8),
          ],
        ),
        actions: [
          ResponsiveDialogActions(
            FilledButton(
              onPressed: isLoading
                  ? null
                  : () async {
                      setState(() {
                        isLoading = true;
                      });

                      final operator = widget.operator;

                      Mix? response;
                      if (widget.mixId != null) {
                        response = await updateMix(
                          widget.mixId!,
                          titleController.text,
                          groupController.text,
                          false,
                          99,
                          operator == null ? [] : [operator],
                        );
                      } else {
                        response = await createMix(
                          titleController.text,
                          groupController.text,
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
          ),
        ],
      ),
    );
  }
}
