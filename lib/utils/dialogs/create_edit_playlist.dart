import 'package:fluent_ui/fluent_ui.dart';

import '../../messages/playlist.pb.dart';

class CreateEditPlaylistDialog extends StatefulWidget {
  final int? playlistId;

  const CreateEditPlaylistDialog({super.key, this.playlistId});

  @override
  CreateEditPlaylistDialogState createState() =>
      CreateEditPlaylistDialogState();
}

class CreateEditPlaylistDialogState extends State<CreateEditPlaylistDialog> {
  final titleController = TextEditingController();
  bool isLoading = false;
  List<String> groupList = ['Favorite'];
  String selectedGroup = 'Favorite';

  PlaylistWithoutCoverIds? playlist;

  @override
  void initState() {
    super.initState();
    fetchGroupList();
    if (widget.playlistId != null) {
      loadPlaylist(widget.playlistId!);
    }
  }

  Future<void> fetchGroupList() async {
    final groups = await getGroupList();
    setState(() {
      groupList = ['Favorite', ...groups];
    });
  }

  Future<void> loadPlaylist(int playlistId) async {
    playlist = await getPlaylistById(playlistId);
    if (playlist != null) {
      titleController.text = playlist!.name;
      selectedGroup = playlist!.group;
    }
    setState(() {});
  }

  @override
  Widget build(BuildContext context) {
    return ContentDialog(
      title: Column(
        children: [
          const SizedBox(height: 16),
          Text(widget.playlistId != null ? 'Edit Playlist' : 'Create Playlist'),
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

                  PlaylistWithoutCoverIds? response;
                  if (widget.playlistId != null) {
                    response = await updatePlaylist(
                      widget.playlistId!,
                      titleController.text,
                      selectedGroup,
                    );
                  } else {
                    response = await createPlaylist(
                      titleController.text,
                      selectedGroup,
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
      ],
    );
  }
}

Future<PlaylistWithoutCoverIds?> showCreateEditPlaylistDialog(
    BuildContext context,
    {int? playlistId}) async {
  return await showDialog<PlaylistWithoutCoverIds?>(
    context: context,
    builder: (context) => CreateEditPlaylistDialog(playlistId: playlistId),
  );
}

Future<List<String>> getGroupList() async {
  final fetchGroupsRequest = FetchPlaylistsGroupSummaryRequest();
  fetchGroupsRequest.sendSignalToRust(); // GENERATED

  // Listen for the response from Rust
  final rustSignal = await PlaylistGroupSummaryResponse.rustSignalStream.first;
  final groups = rustSignal.message.playlistsGroups
      .map((group) => group.groupTitle)
      .toList();

  return groups;
}

Future<PlaylistWithoutCoverIds> getPlaylistById(int playlistId) async {
  final fetchMediaFiles = GetPlaylistByIdRequest(playlistId: playlistId);
  fetchMediaFiles.sendSignalToRust(); // GENERATED

  // Listen for the response from Rust
  final rustSignal = await GetPlaylistByIdResponse.rustSignalStream.first;
  final playlist = rustSignal.message.playlist;

  return playlist;
}

Future<PlaylistWithoutCoverIds> createPlaylist(
    String name, String group) async {
  final createRequest = CreatePlaylistRequest(name: name, group: group);
  createRequest.sendSignalToRust(); // GENERATED

  // Listen for the response from Rust
  final rustSignal = await CreatePlaylistResponse.rustSignalStream.first;
  final response = rustSignal.message;

  return response.playlist;
}

Future<PlaylistWithoutCoverIds> updatePlaylist(
    int playlistId, String name, String group) async {
  final updateRequest = UpdatePlaylistRequest(
    playlistId: playlistId,
    name: name,
    group: group,
  );
  updateRequest.sendSignalToRust(); // GENERATED

  // Listen for the response from Rust
  final rustSignal = await UpdatePlaylistResponse.rustSignalStream.first;
  final response = rustSignal.message;

  return response.playlist;
}
