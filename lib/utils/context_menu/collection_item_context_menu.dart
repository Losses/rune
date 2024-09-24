import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:player/utils/api/fetch_mix_queries_by_mix_id.dart';

import '../../utils/api/get_all_mixes.dart';
import '../../utils/api/add_item_to_mix.dart';
import '../../utils/dialogs/mix/mix_studio.dart';
import '../../utils/dialogs/mix/create_edit_mix.dart';
import '../../utils/dialogs/mix/remove_mix_dialog.dart';
import '../../utils/dialogs/playlist/remove_playlist_dialog.dart';
import '../../utils/dialogs/playlist/create_edit_playlist.dart';

import '../../messages/playback.pb.dart';
import '../../messages/mix.pbserver.dart';

final Map<String, String> typeToOperator = {
  "album": "lib::album",
  "artist": "lib::artist",
  "playlist": "lib::playlist",
  "track": "lib::track",
};

final Map<
    String,
    void Function(
      BuildContext context,
      void Function()? refreshList,
      int id,
    )> typeToEdit = {
  "playlist": (context, refreshList, id) async {
    final result = await showCreateEditPlaylistDialog(context, playlistId: id);

    if (result != null && refreshList != null) {
      refreshList();
    }
  },
  "mix": (context, refreshList, id) async {
    final result = await showMixStudioDialog(context, mixId: id);

    if (result != null && refreshList != null) {
      refreshList();
    }
  },
};

final Map<String, String> typeToEditLabel = {
  "playlist": "Edit Playlist",
  "mix": "Edit Mix",
};

final Map<
    String,
    void Function(
      BuildContext context,
      void Function()? refreshList,
      int id,
    )> typeToRemove = {
  "playlist": (context, refreshList, id) async {
    final result = await showRemovePlaylistDialog(context, id);

    if (result == true && refreshList != null) {
      refreshList();
    }
  },
  "mix": (context, refreshList, id) async {
    final result = await showRemoveMixDialog(context, id);

    if (result == true && refreshList != null) {
      refreshList();
    }
  },
};

final Map<String, String> typeToRemoveLabel = {
  "playlist": "Remove Playlist",
  "mix": "Remove Mix",
};

void openCollectionItemContextMenu(
  Offset localPosition,
  BuildContext context,
  GlobalKey contextAttachKey,
  FlyoutController contextController,
  String type,
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

  final mixes = await getAllMixes();

  contextController.showFlyout(
    position: position,
    builder: (context) => buildCollectionItemContextMenu(
      context,
      type,
      id,
      mixes,
      refreshList,
      readonly,
    ),
  );
}

Widget buildCollectionItemContextMenu(
  BuildContext context,
  String type,
  int id,
  List<MixWithoutCoverIds> mixes, [
  void Function()? refreshList,
  bool? readonly,
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

  Future<List<MixQuery>> mixOrQuery(String type, int id) async {
    final List<(String, String)> queries = type == 'mix'
        ? await fetchMixQueriesByMixId(id)
        : [(typeToOperator[type]!, id.toString())];

    return queries
        .map((x) => MixQuery(operator: x.$1, parameter: x.$2))
        .toList();
  }

  List<MenuFlyoutItemBase> items = [
    MenuFlyoutItem(
      leading: const Icon(Symbols.play_circle),
      text: const Text('Start Playing'),
      onPressed: () async {
        OperatePlaybackWithMixQueryRequest(
          queries: await mixOrQuery(type, id),
          playbackMode: 99,
          hintPosition: -1,
          initialPlaybackId: 0,
          instantlyPlay: true,
          replacePlaylist: true,
        ).sendSignalToRust();
      },
    ),
    MenuFlyoutItem(
      leading: const Icon(Symbols.playlist_add),
      text: const Text('Add to Queue'),
      onPressed: () async {
        OperatePlaybackWithMixQueryRequest(
          queries: await mixOrQuery(type, id),
          playbackMode: 99,
          hintPosition: -1,
          initialPlaybackId: 0,
          instantlyPlay: false,
          replacePlaylist: false,
        ).sendSignalToRust();
      },
    ),
    MenuFlyoutItem(
      leading: const Icon(Symbols.rocket),
      text: const Text('Start Roaming'),
      onPressed: () async {
        final queries = await mixOrQuery(type, id);

        queries.add(MixQuery(operator: "pipe::limit", parameter: "50"));
        queries.add(MixQuery(operator: "pipe::recommend", parameter: "-1"));

        OperatePlaybackWithMixQueryRequest(
          queries: queries,
          playbackMode: 99,
          hintPosition: -1,
          initialPlaybackId: 0,
          instantlyPlay: true,
          replacePlaylist: true,
        ).sendSignalToRust();
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
