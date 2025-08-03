import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/l10n.dart';
import '../../utils/api/get_all_mixes.dart';
import '../../utils/api/add_item_to_mix.dart';
import '../../utils/api/get_all_playlists.dart';
import '../../utils/api/if_analysis_exists.dart';
import '../../utils/api/add_item_to_playlist.dart';
import '../../utils/api/get_parsed_media_file.dart';
import '../../utils/dialogs/mix/create_edit_mix.dart';
import '../../bindings/bindings.dart';
import '../../providers/responsive_providers.dart';

import '../rune_log.dart';
import '../execute_middle_click_action.dart';
import '../api/remove_item_from_playlist.dart';
import '../router/navigation.dart';
import '../router/query_tracks_parameter.dart';
import '../router/router_aware_flyout_controller.dart';
import '../dialogs/playlist/create_edit_playlist.dart';

void openTrackItemContextMenu(
  Offset localPosition,
  BuildContext context,
  GlobalKey contextAttachKey,
  RouterAwareFlyoutController contextController,
  int? positionIndex,
  int fileId,
  int? playlistId,
  void Function()? refreshList,
) async {
  final targetContext = contextAttachKey.currentContext;

  if (targetContext == null) return;
  final box = targetContext.findRenderObject() as RenderBox;
  final position = box.localToGlobal(
    localPosition,
    ancestor: Navigator.of(context).context.findRenderObject(),
  );
  final analyzed = await ifAnalyzeExists(fileId);

  final playlists = await getAllPlaylists();
  final mixes = await getAllMixes();
  final parsedMediaFile = await getParsedMediaFile(fileId);

  if (!context.mounted) return;

  final r = Provider.of<ResponsiveProvider>(context, listen: false);

  final isDock = r.smallerOrEqualTo(DeviceType.dock, false);
  final isBand = r.smallerOrEqualTo(DeviceType.band, false);

  final isMini = isDock || isBand;

  if (isMini) {
    contextController.showFlyout(
      position: position,
      builder: (context) {
        return buildBandScreenTrackItemContextMenu(
          context,
          parsedMediaFile,
          isBand,
        );
      },
    );

    return;
  }

  contextController.showFlyout(
    position: position,
    builder: (context) => buildTrackItemContextMenu(
      context,
      parsedMediaFile,
      playlists,
      mixes,
      analyzed,
      positionIndex,
      playlistId,
      refreshList,
    ),
  );
}

Widget buildTrackItemContextMenu(
  BuildContext context,
  FetchParsedMediaFileResponse item,
  List<Playlist> playlists,
  List<Mix> mixes,
  bool analyzed,
  int? position,
  int? playlistId,
  void Function()? refreshList,
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
        leading: const Icon(Symbols.play_circle),
        text: Text(S.of(context).startPlaying),
        onPressed: () async {
          startPlaying(CollectionType.track, item.file.id, const []);
        },
      ),
      MenuFlyoutItem(
        leading: const Icon(Symbols.playlist_add),
        text: Text(S.of(context).addToQueue),
        onPressed: () async {
          addToQueue(CollectionType.track, item.file.id, const []);
        },
      ),
      MenuFlyoutItem(
        leading: const Icon(Symbols.rocket),
        text: Text(S.of(context).startRoaming),
        onPressed: () async {
          startRoaming(context, CollectionType.track, item.file.id, const []);
        },
      ),
      if (playlistId != null) const MenuFlyoutSeparator(),
      MenuFlyoutItem(
        leading: const Icon(Symbols.delete),
        text: Text(S.of(context).removeFromPlaylist),
        onPressed: () async {
          if (playlistId == null) return;
          if (position == null) return;

          try {
            await removeItemFromPlaylist(playlistId, item.file.id, position);
            if (refreshList != null) {
              refreshList();
            }
          } catch (e) {
            error$(e.toString());
          }
        },
      ),
      const MenuFlyoutSeparator(),
      if (item.artists.length == 1)
        MenuFlyoutItem(
          leading: const Icon(Symbols.face),
          text: Text(S.of(context).goToArtist),
          onPressed: () => {
            $push(
              '/artists/detail',
              arguments: QueryTracksParameter(
                item.artists[0].id,
                item.artists[0].name,
              ),
            )
          },
        ),
      if (item.artists.length > 1)
        MenuFlyoutSubItem(
            leading: const Icon(Symbols.face),
            text: Text(S.of(context).goToArtist),
            items: (context) => item.artists
                .map((x) => MenuFlyoutItem(
                      leading: const Icon(Symbols.face),
                      text: Text(x.name),
                      onPressed: () => {
                        $push(
                          '/artists/detail',
                          arguments: QueryTracksParameter(x.id, x.name),
                        )
                      },
                    ))
                .toList()),
      MenuFlyoutItem(
        leading: const Icon(Symbols.album),
        text: Text(S.of(context).goToAlbum),
        onPressed: () => {
          $push(
            '/albums/detail',
            arguments: QueryTracksParameter(item.album.id, item.album.name),
          )
        },
      ),
      const MenuFlyoutSeparator(),
      MenuFlyoutSubItem(
        leading: const Icon(Symbols.list_alt),
        text: Text(S.of(context).addToPlaylist),
        items: (context) => [
          MenuFlyoutItem(
            leading: const Icon(Symbols.add),
            text: Text(S.of(context).newPlaylist),
            onPressed: () async {
              Flyout.of(context).close();

              final playlist = await showCreateEditPlaylistDialog(
                context,
                item.file.title,
                playlistId: null,
              );

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
        text: Text(S.of(context).addToMix),
        items: (context) => [
          MenuFlyoutItem(
            leading: const Icon(Symbols.add),
            text: Text(S.of(context).newMix),
            onPressed: () async {
              Flyout.of(context).close();

              final playlist = await showCreateEditMixDialog(
                context,
                item.file.title,
                mixId: null,
              );

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

FlyoutContent buildBandScreenTrackItemContextMenu(
  BuildContext context,
  FetchParsedMediaFileResponse item,
  bool isBand,
) {
  List<CommandBarButton> items = [
    CommandBarButton(
      icon: Tooltip(
        message: S.of(context).startPlaying,
        child: const Icon(Symbols.play_circle),
      ),
      onPressed: () async {
        startPlaying(CollectionType.track, item.file.id, const []);
      },
    ),
    CommandBarButton(
      icon: Tooltip(
        message: S.of(context).addToQueue,
        child: const Icon(Symbols.playlist_add),
      ),
      onPressed: () async {
        addToQueue(CollectionType.track, item.file.id, const []);
      },
    ),
    CommandBarButton(
      icon: Tooltip(
        message: S.of(context).startRoaming,
        child: const Icon(Symbols.rocket),
      ),
      onPressed: () async {
        startRoaming(context, CollectionType.track, item.file.id, const []);
      },
    ),
  ];

  return FlyoutContent(
    child: Container(
      constraints: const BoxConstraints(maxHeight: 96, maxWidth: 96),
      child: CommandBar(
        primaryItems: items,
        direction: isBand ? Axis.horizontal : Axis.vertical,
        overflowBehavior: CommandBarOverflowBehavior.scrolling,
      ),
    ),
  );
}
