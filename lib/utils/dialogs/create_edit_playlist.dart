import 'package:fluent_ui/fluent_ui.dart';
import 'package:player/messages/playlist.pb.dart';

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

  PlaylistWithoutCoverIds? playlist;

  @override
  void initState() {
    super.initState();
    if (widget.playlistId != null) {
      loadPlaylist(widget.playlistId!);
    }
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
            child: TextBox(
              controller: groupController,
              enabled: !isLoading,
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
          child: isLoading
              ? const ProgressRing()
              : Text(widget.playlistId != null ? 'Save' : 'Create'),
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
