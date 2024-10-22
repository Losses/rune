import 'package:fluent_ui/fluent_ui.dart';
import 'package:rune/widgets/responsive_dialog_actions.dart';

import '../../../utils/api/update_playlist.dart';
import '../../../utils/api/create_playlist.dart';
import '../../../utils/api/get_playlist_by_id.dart';
import '../../../utils/dialogs/unavailable_dialog_on_band.dart';
import '../../../messages/playlist.pb.dart';
import '../../../messages/collection.pb.dart';

import '../../api/fetch_collection_group_summary_title.dart';

class CreateEditPlaylistDialog extends StatefulWidget {
  final int? playlistId;

  const CreateEditPlaylistDialog({super.key, this.playlistId});

  @override
  CreateEditPlaylistDialogState createState() =>
      CreateEditPlaylistDialogState();
}

class CreateEditPlaylistDialogState extends State<CreateEditPlaylistDialog> {
  final titleController = TextEditingController();
  final groupController = TextEditingController();
  bool isLoading = false;
  List<String> groupList = ['Favorite'];

  Playlist? playlist;

  @override
  void initState() {
    super.initState();
    fetchGroupList();
    if (widget.playlistId != null) {
      loadPlaylist(widget.playlistId!);
    }
  }

  Future<void> fetchGroupList() async {
    final groups =
        await fetchCollectionGroupSummaryTitle(CollectionType.Playlist);
    setState(() {
      groupList = ['Favorite', ...groups];
    });
  }

  Future<void> loadPlaylist(int playlistId) async {
    playlist = await getPlaylistById(playlistId);
    if (playlist != null) {
      titleController.text = playlist!.name;
      groupController.text = playlist!.group;
    }
    setState(() {});
  }

  @override
  Widget build(BuildContext context) {
    return UnavailableDialogOnBand(
      child: ContentDialog(
        title: Column(
          children: [
            const SizedBox(height: 8),
            Text(widget.playlistId != null
                ? 'Edit Playlist'
                : 'Create Playlist'),
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
                items: groupList.map<AutoSuggestBoxItem<String>>((e) {
                  return AutoSuggestBoxItem<String>(
                    value: e,
                    label: e,
                  );
                }).toList(),
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

                      Playlist? response;
                      if (widget.playlistId != null) {
                        response = await updatePlaylist(
                          widget.playlistId!,
                          titleController.text,
                          groupController.text,
                        );
                      } else {
                        response = await createPlaylist(
                          titleController.text,
                          groupController.text,
                        );
                      }

                      setState(() {
                        isLoading = false;
                      });

                      if (!context.mounted) return;
                      Navigator.pop(context, response);
                    },
              child: Text(widget.playlistId != null ? 'Save' : 'Create'),
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
