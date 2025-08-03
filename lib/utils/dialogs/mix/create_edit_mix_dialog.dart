import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/dialogs/clean_group_titles.dart';
import '../../../widgets/no_shortcuts.dart';
import '../../../widgets/responsive_dialog_actions.dart';
import '../../../screens/collection/utils/collection_data_provider.dart';
import '../../../bindings/bindings.dart';
import '../../../utils/l10n.dart';

import '../../api/create_mix.dart';
import '../../api/update_mix.dart';
import '../../api/get_mix_by_id.dart';
import '../../api/fetch_collection_group_summary_title.dart';

import '../unavailable_dialog_on_band.dart';

class CreateEditMixDialog extends StatefulWidget {
  final int? mixId;
  final String? defaultTitle;
  final (String, String)? operator;
  final void Function(Mix?) $close;

  const CreateEditMixDialog({
    super.key,
    this.mixId,
    required this.defaultTitle,
    this.operator,
    required this.$close,
  });

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

    final defaultTitle = widget.defaultTitle;

    if (defaultTitle != null) {
      titleController.text = defaultTitle;
    }

    fetchGroupList();
    if (widget.mixId != null) {
      loadMix(widget.mixId!);
    }
  }

  @override
  void dispose() {
    super.dispose();
    titleController.dispose();
    groupController.dispose();
  }

  Future<void> fetchGroupList() async {
    final groups = await fetchCollectionGroupSummaryTitle(CollectionType.mix);

    if (!mounted) return;

    setState(() {
      groupList = cleanGroupTitles(['Favorite', ...groups]);
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
      $close: widget.$close,
      child: NoShortcuts(
        ContentDialog(
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
                label: S.of(context).title,
                child: TextBox(
                  controller: titleController,
                  enabled: !isLoading,
                ),
              ),
              const SizedBox(height: 16),
              InfoLabel(
                label: S.of(context).group,
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
                  placeholder: S.of(context).selectAGroup,
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

                        CollectionCache().clearType(CollectionType.mix);

                        setState(() {
                          isLoading = false;
                        });

                        if (!context.mounted) return;
                        widget.$close(response);
                      },
                child: Text(widget.mixId != null
                    ? S.of(context).save
                    : S.of(context).create),
              ),
              Button(
                onPressed: isLoading ? null : () => widget.$close(null),
                child: Text(S.of(context).cancel),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
