import 'package:fluent_ui/fluent_ui.dart';
import 'package:go_router/go_router.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/query_list.dart';
import '../../utils/router_extra.dart';
import '../../utils/api/get_all_mixes.dart';
import '../../utils/api/add_item_to_mix.dart';
import '../../utils/api/get_all_playlists.dart';
import '../../utils/api/if_analysis_exists.dart';
import '../../utils/api/add_item_to_playlist.dart';
import '../../utils/api/get_parsed_media_file.dart';
import '../../utils/api/operate_playback_with_mix_query.dart';
import '../../utils/dialogs/mix/create_edit_mix.dart';
import '../../utils/dialogs/no_analysis/show_no_analysis_dialog.dart';
import '../../messages/mix.pbserver.dart';
import '../../messages/media_file.pb.dart';
import '../../messages/playlist.pbserver.dart';

import '../dialogs/playlist/create_edit_playlist.dart';

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
  final analysed = await ifAnalyseExists(fileId);

  final playlists = await getAllPlaylists();
  final mixes = await getAllMixes();
  final parsedMediaFile = await getParsedMediaFile(fileId);

  contextController.showFlyout(
    position: position,
    builder: (context) => buildTrackItemContextMenu(
      context,
      parsedMediaFile,
      playlists,
      mixes,
      analysed,
    ),
  );
}

Widget buildTrackItemContextMenu(
  BuildContext context,
  FetchParsedMediaFileResponse item,
  List<PlaylistWithoutCoverIds> playlists,
  List<MixWithoutCoverIds> mixes,
  bool analysed,
) {
  final List<MenuFlyoutItem> playlistItems = playlists.map((playlist) {
    return MenuFlyoutItem(
      leading: const Icon(Symbols.list_alt),
      text: Text(playlist.name),
      onPressed: () {
        addItemToPlaylist(playlist.id, item.file.id);

        Flyout.of(context).close();
      },
    );
  }).toList();

  final List<MenuFlyoutItem> mixItems =
      mixes.where((x) => !x.locked).map((mixes) {
    return MenuFlyoutItem(
      leading: const Icon(Symbols.magic_button),
      text: Text(mixes.name),
      onPressed: () {
        addItemToMix(
          mixes.id,
          "lib::track",
          item.file.id.toString(),
        );

        Flyout.of(context).close();
      },
    );
  }).toList();

  return MenuFlyout(
    items: [
      MenuFlyoutItem(
        leading: const Icon(Symbols.rocket),
        text: const Text('Start Roaming'),
        onPressed: () async {
          if (!analysed) {
            showNoAnalysisDialog(context);
            return;
          }

          if (context.mounted) {
            await safeOperatePlaybackWithMixQuery(
              context: context,
              queries: QueryList([
                ("lib::track", item.file.id.toString()),
                ("pipe::limit", "50"),
                ("pipe::recommend", "-1")
              ]),
              playbackMode: 99,
              hintPosition: -1,
              initialPlaybackId: 0,
              instantlyPlay: true,
              replacePlaylist: true,
              fallbackFileIds: [],
            );
          }
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
          GoRouter.of(context).replace(
            '/albums/${item.album.id}',
            extra: QueryTracksExtra(item.album.name),
          )
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

              await addItemToPlaylist(playlist.id, item.file.id);
            },
          ),
          if (playlistItems.isNotEmpty) const MenuFlyoutSeparator(),
          ...playlistItems
        ],
      ),
      MenuFlyoutSubItem(
        leading: const Icon(Symbols.magic_button),
        text: const Text('Add to Mix'),
        items: (context) => [
          MenuFlyoutItem(
            leading: const Icon(Symbols.add),
            text: const Text('New Mix'),
            onPressed: () async {
              Flyout.of(context).close();

              final playlist =
                  await showCreateEditMixDialog(context, mixId: null);

              if (playlist == null) return;

              await addItemToMix(
                playlist.id,
                "lib::track",
                item.file.id.toString(),
              );
            },
          ),
          if (mixItems.isNotEmpty) const MenuFlyoutSeparator(),
          ...mixItems
        ],
      ),
    ],
  );
}
