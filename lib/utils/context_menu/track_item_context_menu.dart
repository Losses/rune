import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/api/get_all_mixes.dart';
import '../../utils/api/add_item_to_mix.dart';
import '../../utils/api/get_all_playlists.dart';
import '../../utils/api/if_analysis_exists.dart';
import '../../utils/api/add_item_to_playlist.dart';
import '../../utils/api/get_parsed_media_file.dart';
import '../../utils/dialogs/mix/create_edit_mix.dart';
import '../../messages/all.dart';
import '../../providers/responsive_providers.dart';

import '../execute_middle_click_action.dart';
import '../router/navigation.dart';
import '../router/query_tracks_parameter.dart';
import '../dialogs/playlist/create_edit_playlist.dart';

void openTrackItemContextMenu(
  Offset localPosition,
  BuildContext context,
  GlobalKey contextAttachKey,
  FlyoutController contextController,
  int fileId,
) async {
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
      analysed,
    ),
  );
}

Widget buildTrackItemContextMenu(
  BuildContext context,
  FetchParsedMediaFileResponse item,
  List<Playlist> playlists,
  List<Mix> mixes,
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
        leading: const Icon(Symbols.play_circle),
        text: const Text('Start Playing'),
        onPressed: () async {
          startPlaying(CollectionType.Track, item.file.id, const []);
        },
      ),
      MenuFlyoutItem(
        leading: const Icon(Symbols.playlist_add),
        text: const Text('Add to Queue'),
        onPressed: () async {
          addToQueue(CollectionType.Track, item.file.id, const []);
        },
      ),
      MenuFlyoutItem(
        leading: const Icon(Symbols.rocket),
        text: const Text('Start Roaming'),
        onPressed: () async {
          startRoaming(context, CollectionType.Track, item.file.id, const []);
        },
      ),
      const MenuFlyoutSeparator(),
      if (item.artists.length == 1)
        MenuFlyoutItem(
          leading: const Icon(Symbols.face),
          text: const Text('Go to Artist'),
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
            text: const Text('Go to Artist'),
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
        text: const Text('Go to Album'),
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

FlyoutContent buildBandScreenTrackItemContextMenu(
  BuildContext context,
  FetchParsedMediaFileResponse item,
  bool isBand,
) {
  List<CommandBarButton> items = [
    CommandBarButton(
      icon: const Tooltip(
        message: 'Start Playing',
        child: Icon(Symbols.play_circle),
      ),
      onPressed: () async {
        startPlaying(CollectionType.Track, item.file.id, const []);
      },
    ),
    CommandBarButton(
      icon: const Tooltip(
        message: 'Add to Queue',
        child: Icon(Symbols.playlist_add),
      ),
      onPressed: () async {
        addToQueue(CollectionType.Track, item.file.id, const []);
      },
    ),
    CommandBarButton(
      icon: const Tooltip(
        message: 'Start Roaming',
        child: Icon(Symbols.rocket),
      ),
      onPressed: () async {
        startRoaming(context, CollectionType.Track, item.file.id, const []);
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
