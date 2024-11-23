import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/api/get_all_mixes.dart';
import '../../utils/api/add_item_to_mix.dart';
import '../../utils/dialogs/mix/mix_studio.dart';
import '../../utils/dialogs/mix/create_edit_mix.dart';
import '../../utils/dialogs/mix/remove_mix_dialog.dart';
import '../../utils/dialogs/playlist/create_edit_playlist.dart';
import '../../utils/dialogs/playlist/remove_playlist_dialog.dart';
import '../../messages/mix.pbserver.dart';
import '../../messages/collection.pb.dart';
import '../../providers/responsive_providers.dart';
import '../../utils/l10n.dart';

import '../execute_middle_click_action.dart';

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
    final result =
        await showCreateEditPlaylistDialog(context, "", playlistId: id);

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

Map<CollectionType, String> typeToEditLabel(BuildContext context) => {
      CollectionType.Playlist: S.of(context).editPlaylist,
      CollectionType.Mix: S.of(context).editMix,
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

Map<CollectionType, String> typeToRemoveLabel(BuildContext context) => {
      CollectionType.Playlist: S.of(context).removePlaylist,
      CollectionType.Mix: S.of(context).removeMix,
    };

void openCollectionItemContextMenu(
  Offset localPosition,
  BuildContext context,
  GlobalKey contextAttachKey,
  FlyoutController contextController,
  CollectionType type,
  int id,
  String title, [
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

  if (!context.mounted) return;

  final r = Provider.of<ResponsiveProvider>(context, listen: false);

  final isDock = r.smallerOrEqualTo(DeviceType.dock, false);
  final isBand = r.smallerOrEqualTo(DeviceType.band, false);

  final isMini = isDock || isBand;

  if (isMini) {
    contextController.showFlyout(
      position: position,
      builder: (context) {
        return buildBandScreenCollectionItemContextMenu(
          context,
          type,
          id,
          isBand,
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
      title,
      mixes,
      refreshList,
      readonly,
    ),
  );
}

MenuFlyout buildLargeScreenCollectionItemContextMenu(
  BuildContext context,
  CollectionType type,
  int id,
  String title,
  List<Mix> mixes, [
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
      text: Text(S.of(context).startPlaying),
      onPressed: () async {
        startPlaying(type, id, fallbackFileIds);
      },
    ),
    MenuFlyoutItem(
      leading: const Icon(Symbols.playlist_add),
      text: Text(S.of(context).addToQueue),
      onPressed: () async {
        addToQueue(type, id, fallbackFileIds);
      },
    ),
    MenuFlyoutItem(
      leading: const Icon(Symbols.rocket),
      text: Text(S.of(context).startRoaming),
      onPressed: () async {
        startRoaming(context, type, id, fallbackFileIds);
      },
    ),
  ];

  if (edit != null) {
    items.add(const MenuFlyoutSeparator());
    items.add(
      MenuFlyoutItem(
        leading: const Icon(Symbols.edit),
        text: Text(typeToEditLabel(context)[type] ?? S.of(context).edit),
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
        text: Text(typeToRemoveLabel(context)[type] ?? S.of(context).remove),
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
        text: Text(S.of(context).addToMix),
        items: (context) => [
          MenuFlyoutItem(
            leading: const Icon(Symbols.add),
            text: Text(S.of(context).newMix),
            onPressed: () async {
              Flyout.of(context).close();

              await showCreateEditMixDialog(
                context,
                title,
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
  bool isBand, [
  List<int> fallbackFileIds = const [],
]) {
  List<CommandBarButton> items = [
    CommandBarButton(
      icon: Tooltip(
        message: S.of(context).startPlaying,
        child: const Icon(Symbols.play_circle),
      ),
      onPressed: () async {
        startPlaying(type, id, fallbackFileIds);
      },
    ),
    CommandBarButton(
      icon: Tooltip(
        message: S.of(context).addToQueue,
        child: const Icon(Symbols.playlist_add),
      ),
      onPressed: () async {
        addToQueue(type, id, fallbackFileIds);
      },
    ),
    CommandBarButton(
      icon: Tooltip(
        message: S.of(context).startRoaming,
        child: const Icon(Symbols.rocket),
      ),
      onPressed: () async {
        startRoaming(context, type, id, fallbackFileIds);
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
