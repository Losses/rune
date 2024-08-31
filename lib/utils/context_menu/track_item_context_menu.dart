import 'package:fluent_ui/fluent_ui.dart';
import 'package:go_router/go_router.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../messages/media_file.pb.dart';
import '../../messages/playlist.pbserver.dart';
import '../../messages/recommend.pbserver.dart';
import '../../utils/router_extra.dart';
import '../../utils/dialogs/create_edit_playlist.dart';

void openTrackItemContextMenu(
    Offset localPosition,
    BuildContext context,
    GlobalKey contextAttachKey,
    FlyoutController contextController,
    int fileId) async {
  final targetContext = contextAttachKey.currentContext;

  if (targetContext == null) return;
  final box = targetContext.findRenderObject() as RenderBox;
  final position = box.localToGlobal(
    localPosition,
    ancestor: Navigator.of(context).context.findRenderObject(),
  );

  final playlists = await getAllPlaylists();
  final parsedMediaFile = await getParsedMediaFile(fileId);

  contextController.showFlyout(
    position: position,
    builder: (context) =>
        buildTrackItemContextMenu(context, parsedMediaFile, playlists),
  );
}

Widget buildTrackItemContextMenu(
    BuildContext context,
    FetchParsedMediaFileResponse item,
    List<PlaylistWithoutCoverIds> playlists) {
  final List<MenuFlyoutItem> items = playlists.map((playlist) {
    return MenuFlyoutItem(
      leading: const Icon(Symbols.list_alt),
      text: Text(playlist.name),
      onPressed: () {
        final fetchMediaFiles = AddItemToPlaylistRequest(
          playlistId: playlist.id,
          mediaFileId: item.file.id,
          position: null,
        );
        fetchMediaFiles.sendSignalToRust(); // GENERATED

        Flyout.of(context).close();
      },
    );
  }).toList();

  return MenuFlyout(
    items: [
      MenuFlyoutItem(
        leading: const Icon(Symbols.rocket),
        text: const Text('Start Roaming'),
        onPressed: () => {
          RecommendAndPlayRequest(fileId: item.file.id)
              .sendSignalToRust() // GENERATED
        },
      ),
      if (item.artists.length == 1)
        MenuFlyoutItem(
          leading: const Icon(Symbols.face),
          text: const Text('Go to Artist'),
          onPressed: () => {
            GoRouter.of(context).replace('/artists/${item.artists[0].id}',
                extra: QueryTracksExtra(item.artists[0].name))
          },
        ),
      if (item.artists.length > 1)
        MenuFlyoutSubItem(
            leading: const Icon(Symbols.face),
            text: const Text('Go to Artist'),
            items: (context) => item.artists
                .map((x) => MenuFlyoutItem(
                      leading: const Icon(Symbols.face),
                      text: Text(x.name),
                      onPressed: () => {
                        GoRouter.of(context).replace('/artists/${x.id}',
                            extra: QueryTracksExtra(x.name))
                      },
                    ))
                .toList()),
      MenuFlyoutItem(
        leading: const Icon(Symbols.album),
        text: const Text('Go to Album'),
        onPressed: () => {
          GoRouter.of(context).replace('/albums/${item.album.id}',
              extra: QueryTracksExtra(item.album.name))
        },
      ),
      const MenuFlyoutSeparator(),
      MenuFlyoutSubItem(
        leading: const Icon(Symbols.list_alt),
        text: const Text('Add to Playlist'),
        items: (context) => [
          MenuFlyoutItem(
            leading: const Icon(Symbols.add),
            text: const Text('New Playlist'),
            onPressed: () async {
              Flyout.of(context).close();

              final playlist =
                  await showCreateEditPlaylistDialog(context, playlistId: null);

              if (playlist == null) return;

              final fetchMediaFiles = AddItemToPlaylistRequest(
                playlistId: playlist.id,
                mediaFileId: item.file.id,
                position: null,
              );
              fetchMediaFiles.sendSignalToRust(); // GENERATED

              await AddItemToPlaylistResponse.rustSignalStream.first;
            },
          ),
          if (items.isNotEmpty) const MenuFlyoutSeparator(),
          ...items
        ],
      ),
    ],
  );
}

Future<List<PlaylistWithoutCoverIds>> getAllPlaylists() async {
  final fetchRequest = FetchAllPlaylistsRequest();
  fetchRequest.sendSignalToRust(); // GENERATED

  // Listen for the response from Rust
  final rustSignal = await FetchAllPlaylistsResponse.rustSignalStream.first;
  final response = rustSignal.message;

  return response.playlists;
}

Future<FetchParsedMediaFileResponse> getParsedMediaFile(int fileId) async {
  final fetchRequest = FetchParsedMediaFileRequest(id: fileId);
  fetchRequest.sendSignalToRust(); // GENERATED

  final rustSignal = await FetchParsedMediaFileResponse.rustSignalStream.first;
  final response = rustSignal.message;

  return response;
}
