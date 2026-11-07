import 'dart:io';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:path_provider/path_provider.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:fast_file_picker/fast_file_picker.dart';
import 'package:file_selector/file_selector.dart' show XTypeGroup;

import '../../utils/api/get_all_mixes.dart';
import '../../utils/api/add_item_to_mix.dart';
import '../../utils/dialogs/mix/mix_studio.dart';
import '../../utils/dialogs/mix/create_edit_mix.dart';
import '../../utils/dialogs/mix/remove_mix_dialog.dart';
import '../../utils/dialogs/playlist/create_edit_playlist.dart';
import '../../utils/dialogs/playlist/remove_playlist_dialog.dart';
import '../../bindings/bindings.dart';
import '../../providers/responsive_providers.dart';
import '../../providers/library_path.dart';
import '../../utils/l10n.dart';

import '../build_query.dart';
import '../api/request_failure.dart';
import '../dialogs/information/error.dart';
import '../build_collection_query.dart';
import '../execute_middle_click_action.dart';
import '../dialogs/export_cover_wall/show_export_cover_wall_dialog.dart';
import '../dialogs/play_on/play_on_dialog.dart';

import '../router/router_aware_flyout_controller.dart';
import 'utils/build_m3u8.dart';
import 'offline_selection.dart';

Future<void> _handleCollectionRequest(
  BuildContext context,
  Future<void> Function() action,
) async {
  try {
    await action();
  } on RequestFailure catch (error) {
    if (!context.mounted) return;
    await showErrorDialog(
      context: context,
      title: S.of(context).unknownError,
      errorMessage: requestFailureMessage(S.of(context), error),
    );
  }
}

final Map<
  CollectionType,
  Future<void> Function(
    BuildContext context,
    void Function()? refreshList,
    int id,
  )
>
typeToEdit = {
  CollectionType.playlist: (context, refreshList, id) async {
    final result = await showCreateEditPlaylistDialog(
      context,
      "",
      playlistId: id,
    );

    if (result != null && refreshList != null) {
      refreshList();
    }
  },
  CollectionType.mix: (context, refreshList, id) async {
    final result = await showMixStudioDialog(context, mixId: id);

    if (result != null && refreshList != null) {
      refreshList();
    }
  },
};

Map<CollectionType, String> typeToEditLabel(BuildContext context) => {
  CollectionType.playlist: S.of(context).editPlaylist,
  CollectionType.mix: S.of(context).editMix,
};

final Map<
  CollectionType,
  Future<void> Function(
    BuildContext context,
    void Function()? refreshList,
    int id,
  )
>
typeToRemove = {
  CollectionType.playlist: (context, refreshList, id) async {
    final result = await showRemovePlaylistDialog(context, id);

    if (result == true && refreshList != null) {
      refreshList();
    }
  },
  CollectionType.mix: (context, refreshList, id) async {
    final result = await showRemoveMixDialog(context, id);

    if (result == true && refreshList != null) {
      refreshList();
    }
  },
};

Map<CollectionType, String> typeToRemoveLabel(BuildContext context) => {
  CollectionType.playlist: S.of(context).removePlaylist,
  CollectionType.mix: S.of(context).removeMix,
};

void openCollectionItemContextMenu(
  Offset localPosition,
  BuildContext context,
  GlobalKey contextAttachKey,
  RouterAwareFlyoutController contextController,
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

  late final List<Mix> mixes;
  try {
    mixes = await getAllMixes();
  } on RequestFailure catch (error) {
    if (context.mounted) {
      await showErrorDialog(
        context: context,
        title: S.of(context).unknownError,
        errorMessage: requestFailureMessage(S.of(context), error),
      );
    }
    return;
  }
  if (!context.mounted) return;

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
  final libraryPath = context.read<LibraryPathProvider>().currentPath;

  final List<MenuFlyoutItem> mixItems = mixes.where((x) => !x.locked).map((
    mix,
  ) {
    return MenuFlyoutItem(
      leading: const Icon(Symbols.magic_button),
      text: Text(mix.name),
      onPressed: () {
        final host = Navigator.of(context).context;
        Flyout.of(context).close();
        _handleCollectionRequest(
          host,
          () => addItemToMix(mix.id, operator ?? "lib::unknown", id.toString()),
        );
      },
    );
  }).toList();

  List<MenuFlyoutItemBase> items = [
    if (supportsOfflineSelection(context))
      offlineSelectionMenu(context, type, id),
    MenuFlyoutItem(
      leading: const Icon(Symbols.play_circle),
      text: Text(S.of(context).startPlaying),
      onPressed: () async {
        await _handleCollectionRequest(
          context,
          () => startPlaying(type, id, fallbackFileIds),
        );
      },
    ),
    if (libraryPath != null)
      MenuFlyoutItem(
        leading: const Icon(Symbols.cast),
        text: Text(S.of(context).playOn),
        onPressed: () async {
          final host = Navigator.of(context).context;
          Flyout.of(context).close();
          await _handleCollectionRequest(host, () async {
            await showCastOnDialog(
              host,
              libraryPath: libraryPath,
              cast: (candidate) => castCollectionWithSettings(
                type,
                id,
                fallbackFileIds,
                candidate.fingerprint,
              ),
            );
          });
        },
      ),
    MenuFlyoutItem(
      leading: const Icon(Symbols.playlist_add),
      text: Text(S.of(context).addToQueue),
      onPressed: () async {
        await _handleCollectionRequest(
          context,
          () => addToQueue(type, id, fallbackFileIds),
        );
      },
    ),
    MenuFlyoutItem(
      leading: const Icon(Symbols.rocket),
      text: Text(S.of(context).startRoaming),
      onPressed: () async {
        await _handleCollectionRequest(
          context,
          () => startRoaming(context, type, id, fallbackFileIds),
        );
      },
    ),
  ];

  if (edit != null && !context.read<LibraryPathProvider>().isReadOnlyReplica) {
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

  if (remove != null &&
      !context.read<LibraryPathProvider>().isReadOnlyReplica) {
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

  if (operator != null &&
      !context.read<LibraryPathProvider>().isReadOnlyReplica) {
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
          ...mixItems,
        ],
      ),
    );
  }

  if (Platform.isWindows || Platform.isMacOS || Platform.isLinux) {
    items.add(const MenuFlyoutSeparator());
    items.add(
      MenuFlyoutSubItem(
        leading: const Icon(Symbols.upload),
        text: Text(S.of(context).exportTracks),
        items: (context) => [
          MenuFlyoutItem(
            leading: const Icon(Symbols.list_alt),
            text: Text(S.of(context).exportM3u8),
            onPressed: () async {
              final host = Navigator.of(context).context;
              Flyout.of(context).close();

              final Directory appDocumentsDir =
                  await getApplicationDocumentsDirectory();

              final String? path = await FastFilePicker.pickSaveFile(
                suggestedName: '$title.m3u8',
                initialDirectory: appDocumentsDir.path,
                acceptedTypeGroups: const [
                  XTypeGroup(label: 'playlist', extensions: <String>['m3u8']),
                ],
              );

              if (path == null) return;

              if (!host.mounted) return;
              await _handleCollectionRequest(host, () async {
                final playlist = await buildM3u8(type, id);
                final file = File(path);
                await file.writeAsString(playlist);
              });
            },
          ),
          MenuFlyoutItem(
            leading: const Icon(Symbols.wall_art),
            text: Text(S.of(context).exportCoverWall),
            onPressed: () async {
              final host = Navigator.of(context).context;
              Flyout.of(context).close();
              await _handleCollectionRequest(host, () async {
                final queries = await buildQuery(type, id);
                if (!host.mounted) return;
                showExportCoverWallDialog(host, queries, title);
              });
            },
          ),
        ],
      ),
    );
  }

  return MenuFlyout(items: items);
}

FlyoutContent buildBandScreenCollectionItemContextMenu(
  BuildContext context,
  CollectionType type,
  int id,
  bool isBand, [
  List<int> fallbackFileIds = const [],
]) {
  final libraryPath = context.read<LibraryPathProvider>().currentPath;
  List<CommandBarButton> items = [
    CommandBarButton(
      icon: Tooltip(
        message: S.of(context).startPlaying,
        child: const Icon(Symbols.play_circle),
      ),
      onPressed: () async {
        await _handleCollectionRequest(
          context,
          () => startPlaying(type, id, fallbackFileIds),
        );
      },
    ),
    if (libraryPath != null)
      CommandBarButton(
        icon: Tooltip(
          message: S.of(context).playOn,
          child: const Icon(Symbols.cast),
        ),
        onPressed: () async {
          final host = Navigator.of(context).context;
          await _handleCollectionRequest(host, () async {
            await showCastOnDialog(
              host,
              libraryPath: libraryPath,
              cast: (candidate) => castCollectionWithSettings(
                type,
                id,
                fallbackFileIds,
                candidate.fingerprint,
              ),
            );
          });
        },
      ),
    CommandBarButton(
      icon: Tooltip(
        message: S.of(context).addToQueue,
        child: const Icon(Symbols.playlist_add),
      ),
      onPressed: () async {
        await _handleCollectionRequest(
          context,
          () => addToQueue(type, id, fallbackFileIds),
        );
      },
    ),
    CommandBarButton(
      icon: Tooltip(
        message: S.of(context).startRoaming,
        child: const Icon(Symbols.rocket),
      ),
      onPressed: () async {
        await _handleCollectionRequest(
          context,
          () => startRoaming(context, type, id, fallbackFileIds),
        );
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
