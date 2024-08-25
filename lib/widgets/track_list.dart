import 'dart:math';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:go_router/go_router.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:infinite_scroll_pagination/infinite_scroll_pagination.dart';

import '../../../utils/platform.dart';
import '../../../utils/router_extra.dart';
import '../../../utils/dialogs/create_edit_playlist.dart';
import '../../../widgets/cover_art.dart';
import '../../../widgets/smooth_horizontal_scroll.dart';
import '../../../messages/playlist.pb.dart';
import '../../../messages/playback.pb.dart';
import '../../../messages/media_file.pb.dart';
import '../../../messages/recommend.pbserver.dart';

class TrackList extends StatelessWidget {
  final PagingController<int, MediaFile> pagingController;

  const TrackList({super.key, required this.pagingController});

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.all(12),
      child: LayoutBuilder(builder: (context, constraints) {
        const double gapSize = 8;
        const double cellSize = 64;

        final int rows = (constraints.maxHeight / (cellSize + gapSize)).floor();
        final double finalHeight = rows * (cellSize + gapSize) - gapSize;

        return SmoothHorizontalScroll(
          builder: (context, scrollController) => SizedBox(
            height: finalHeight,
            child: PagedGridView<int, MediaFile>(
              pagingController: pagingController,
              scrollDirection: Axis.horizontal,
              scrollController: scrollController,
              gridDelegate: SliverGridDelegateWithFixedCrossAxisCount(
                crossAxisCount: rows,
                mainAxisSpacing: gapSize,
                crossAxisSpacing: gapSize,
                childAspectRatio: 1 / 4,
              ),
              builderDelegate: PagedChildBuilderDelegate<MediaFile>(
                itemBuilder: (context, item, index) => TrackListItem(
                  index: index,
                  item: item,
                ),
              ),
            ),
          ),
        );
      }),
    );
  }
}

class TrackListItem extends StatelessWidget {
  final MediaFile item;
  final int index;

  final contextController = FlyoutController();
  final contextAttachKey = GlobalKey();

  TrackListItem({
    super.key,
    required this.index,
    required this.item,
  });

  void openContextMenu(
      Offset localPosition, BuildContext context, int fileId) async {
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
          buildContextMenu(context, parsedMediaFile, playlists),
    );
  }

  @override
  Widget build(BuildContext context) {
    Typography typography = FluentTheme.of(context).typography;

    return GestureDetector(
        onSecondaryTapUp: isDesktop
            ? (d) {
                openContextMenu(d.localPosition, context, item.id);
              }
            : null,
        onLongPressEnd: isDesktop
            ? null
            : (d) {
                openContextMenu(d.localPosition, context, item.id);
              },
        child: FlyoutTarget(
            key: contextAttachKey,
            controller: contextController,
            child: Button(
                style: const ButtonStyle(
                    padding: WidgetStatePropertyAll(EdgeInsets.all(0))),
                onPressed: () =>
                    PlayFileRequest(fileId: item.id).sendSignalToRust(),
                child: ClipRRect(
                    borderRadius: BorderRadius.circular(3),
                    child: LayoutBuilder(
                      builder: (context, constraints) {
                        final size =
                            min(constraints.maxWidth, constraints.maxHeight);
                        return Row(
                          children: [
                            CoverArt(
                              fileId: item.id,
                              size: size,
                            ),
                            Expanded(
                                child: Padding(
                              padding: const EdgeInsets.all(8),
                              child: Column(
                                crossAxisAlignment: CrossAxisAlignment.start,
                                mainAxisAlignment: MainAxisAlignment.center,
                                children: [
                                  Text(
                                    item.title,
                                    overflow: TextOverflow.ellipsis,
                                  ),
                                  const SizedBox(height: 4),
                                  Text(
                                    item.artist,
                                    style: typography.caption?.apply(
                                        color: typography.caption?.color
                                            ?.withAlpha(117)),
                                    overflow: TextOverflow.ellipsis,
                                  ),
                                ],
                              ),
                            ))
                          ],
                        );
                      },
                      // GENERATED,
                    )))));
  }
}

Widget buildContextMenu(BuildContext context, FetchParsedMediaFileResponse item,
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
