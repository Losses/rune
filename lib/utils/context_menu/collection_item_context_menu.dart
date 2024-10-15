import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:rune/messages/collection.pb.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/utils/api/fetch_mix_queries_by_mix_id.dart';
import 'package:rune/utils/build_collection_query.dart';
import 'package:provider/provider.dart';

import '../../utils/query_list.dart';
import '../../utils/api/get_all_mixes.dart';
import '../../utils/api/add_item_to_mix.dart';
import '../../utils/api/operate_playback_with_mix_query.dart';
import '../../utils/dialogs/mix/mix_studio.dart';
import '../../utils/dialogs/mix/create_edit_mix.dart';
import '../../utils/dialogs/mix/remove_mix_dialog.dart';
import '../../utils/dialogs/playlist/create_edit_playlist.dart';
import '../../utils/dialogs/playlist/remove_playlist_dialog.dart';

import '../../messages/mix.pbserver.dart';

final Map<CollectionType, String> typeToOperator = {
  CollectionType.Album: "lib::album",
  CollectionType.Artist: "lib::artist",
  CollectionType.Playlist: "lib::playlist",
  CollectionType.Track: "lib::track",
};

final Map<
    CollectionType,
    Future<void> Function(
      BuildContext context,
      void Function()? refreshList,
      int id,
    )> typeToEdit = {
  CollectionType.Playlist: (context, refreshList, id) async {
    final result = await showCreateEditPlaylistDialog(context, playlistId: id);

    if (result != null && refreshList != null) {
      refreshList();
    }
  },
  CollectionType.Mix: (context, refreshList, id) async {
    final result = await showMixStudioDialog(context, mixId: id);

    if (result != null && refreshList != null) {
      refreshList();
    }
  },
};

final Map<CollectionType, String> typeToEditLabel = {
  CollectionType.Playlist: "Edit Playlist",
  CollectionType.Mix: "Edit Mix",
};

final Map<
    CollectionType,
    Future<void> Function(
      BuildContext context,
      void Function()? refreshList,
      int id,
    )> typeToRemove = {
  CollectionType.Playlist: (context, refreshList, id) async {
    final result = await showRemovePlaylistDialog(context, id);

    if (result == true && refreshList != null) {
      refreshList();
    }
  },
  CollectionType.Mix: (context, refreshList, id) async {
    final result = await showRemoveMixDialog(context, id);

    if (result == true && refreshList != null) {
      refreshList();
    }
  },
};

final Map<CollectionType, String> typeToRemoveLabel = {
  CollectionType.Playlist: "Remove Playlist",
  CollectionType.Mix: "Remove Mix",
};

void openCollectionItemContextMenu(
  Offset localPosition,
  BuildContext context,
  GlobalKey contextAttachKey,
  FlyoutController contextController,
  CollectionType type,
  int id, [
  void Function()? refreshList,
  bool? readonly,
]) async {
  final targetContext = contextAttachKey.currentContext;

  if (targetContext == null) return;
  final box = targetContext.findRenderObject() as RenderBox;
  final position = box.localToGlobal(
    localPosition,
    ancestor: Navigator.of(context).context.findRenderObject(),
  );

  final queries = type == CollectionType.Mix
      ? await fetchMixQueriesByMixId(id)
      : buildCollectionQuery(type, id);

  if (!context.mounted) return;

  final isBand = Provider.of<ResponsiveProvider>(context, listen: false)
      .smallerOrEqualTo(DeviceType.band);

  if (isBand) {
    contextController.showFlyout(
      position: position,
      builder: (context) {
        return buildBandScreenCollectionItemContextMenu(
          context,
          type,
          id,
          queries,
        );
      },
    );

    return;
  }

  final mixes = await getAllMixes();

  contextController.showFlyout(
    position: position,
    builder: (context) => buildLargeScreenCollectionItemContextMenu(
      context,
      type,
      id,
      mixes,
      queries,
      refreshList,
      readonly,
    ),
  );
}

MenuFlyout buildLargeScreenCollectionItemContextMenu(
  BuildContext context,
  CollectionType type,
  int id,
  List<Mix> mixes,
  List<(String, String)> queries, [
  void Function()? refreshList,
  bool? readonly,
  List<int> fallbackFileIds = const [],
]) {
  final operator = typeToOperator[type];
  final edit = typeToEdit[type];
  final remove = typeToRemove[type];

  final List<MenuFlyoutItem> mixItems =
      mixes.where((x) => !x.locked).map((mix) {
    return MenuFlyoutItem(
      leading: const Icon(Symbols.magic_button),
      text: Text(mix.name),
      onPressed: () {
        addItemToMix(
          mix.id,
          operator ?? "lib::unknown",
          id.toString(),
        );

        Flyout.of(context).close();
      },
    );
  }).toList();

  List<MenuFlyoutItemBase> items = [
    MenuFlyoutItem(
      leading: const Icon(Symbols.play_circle),
      text: const Text('Start Playing'),
      onPressed: () async {
        operatePlaybackWithMixQuery(
          queries: QueryList(queries),
          playbackMode: 99,
          hintPosition: -1,
          initialPlaybackId: 0,
          instantlyPlay: true,
          replacePlaylist: true,
          fallbackFileIds: fallbackFileIds,
        );
      },
    ),
    MenuFlyoutItem(
      leading: const Icon(Symbols.playlist_add),
      text: const Text('Add to Queue'),
      onPressed: () async {
        operatePlaybackWithMixQuery(
          queries: QueryList(queries),
          playbackMode: 99,
          hintPosition: -1,
          initialPlaybackId: 0,
          instantlyPlay: false,
          replacePlaylist: false,
          fallbackFileIds: [],
        );
      },
    ),
    MenuFlyoutItem(
      leading: const Icon(Symbols.rocket),
      text: const Text('Start Roaming'),
      onPressed: () async {
        final q = QueryList([
          ...queries,
          ("pipe::limit", "50"),
          ("pipe::recommend", "-1"),
        ]);

        if (context.mounted) {
          await safeOperatePlaybackWithMixQuery(
            context: context,
            queries: q,
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
  ];

  if (edit != null) {
    items.add(const MenuFlyoutSeparator());
    items.add(
      MenuFlyoutItem(
        leading: const Icon(Symbols.edit),
        text: Text(typeToEditLabel[type] ?? 'Edit'),
        onPressed: readonly == true
            ? null
            : () {
                edit(context, refreshList, id);
              },
      ),
    );
  }

  if (remove != null) {
    items.add(
      MenuFlyoutItem(
        leading: const Icon(Symbols.delete),
        text: Text(typeToRemoveLabel[type] ?? 'Remove'),
        onPressed: readonly == true
            ? null
            : () {
                remove(context, refreshList, id);
              },
      ),
    );
  }

  if (operator != null) {
    items.add(const MenuFlyoutSeparator());
    items.add(
      MenuFlyoutSubItem(
        leading: const Icon(Symbols.magic_button),
        text: const Text('Add to Mix'),
        items: (context) => [
          MenuFlyoutItem(
            leading: const Icon(Symbols.add),
            text: const Text('New Mix'),
            onPressed: () async {
              Flyout.of(context).close();

              await showCreateEditMixDialog(
                context,
                mixId: null,
                operator: (operator, id.toString()),
              );
            },
          ),
          if (mixItems.isNotEmpty) const MenuFlyoutSeparator(),
          ...mixItems
        ],
      ),
    );
  }

  return MenuFlyout(
    items: items,
  );
}

FlyoutContent buildBandScreenCollectionItemContextMenu(
  BuildContext context,
  CollectionType type,
  int id,
  List<(String, String)> queries, [
  List<int> fallbackFileIds = const [],
]) {
  List<CommandBarButton> items = [
    CommandBarButton(
      icon: const Tooltip(
        message: 'Start Playing',
        child: Icon(Symbols.play_circle),
      ),
      onPressed: () async {
        operatePlaybackWithMixQuery(
          queries: QueryList(queries),
          playbackMode: 99,
          hintPosition: -1,
          initialPlaybackId: 0,
          instantlyPlay: true,
          replacePlaylist: true,
          fallbackFileIds: fallbackFileIds,
        );
      },
    ),
    CommandBarButton(
      icon: const Tooltip(
        message: 'Add to Queue',
        child: Icon(Symbols.playlist_add),
      ),
      onPressed: () async {
        operatePlaybackWithMixQuery(
          queries: QueryList(queries),
          playbackMode: 99,
          hintPosition: -1,
          initialPlaybackId: 0,
          instantlyPlay: false,
          replacePlaylist: false,
          fallbackFileIds: [],
        );
      },
    ),
    CommandBarButton(
      icon: const Tooltip(
        message: 'Start Roaming',
        child: Icon(Symbols.rocket),
      ),
      onPressed: () async {
        final q = QueryList([
          ...queries,
          ("pipe::limit", "50"),
          ("pipe::recommend", "-1"),
        ]);

        if (context.mounted) {
          await safeOperatePlaybackWithMixQuery(
            context: context,
            queries: q,
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
  ];

  return FlyoutContent(
    child: Container(
      constraints: const BoxConstraints(maxHeight: 96),
      child: CommandBar(
        primaryItems: items,
        direction: Axis.vertical,
        overflowBehavior: CommandBarOverflowBehavior.scrolling,
      ),
    ),
  );
}
