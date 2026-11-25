import 'dart:io';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:path_provider/path_provider.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:file_selector/file_selector.dart' show XTypeGroup;

import '../../utils/api/get_all_mixes.dart';
import '../../utils/api/add_item_to_mix.dart';
import '../../utils/api/get_all_playlists.dart';
import '../../utils/api/add_item_to_playlist.dart';
import '../../utils/dialogs/mix/mix_studio.dart';
import '../../utils/dialogs/mix/create_edit_mix.dart';
import '../../utils/dialogs/mix/remove_mix_dialog.dart';
import '../../utils/dialogs/playlist/create_edit_playlist.dart';
import '../../utils/dialogs/playlist/remove_playlist_dialog.dart';
import '../../bindings/bindings.dart';
import '../../providers/responsive_providers.dart';
import '../../providers/library_open_state.dart';
import '../../providers/request_capabilities.dart';
import '../../utils/l10n.dart';
import '../../utils/query_list.dart';

import '../build_query.dart';
import '../api/request_failure.dart';
import '../dialogs/information/error.dart';
import '../build_collection_query.dart';
import '../execute_middle_click_action.dart';
import '../dialogs/export_cover_wall/show_export_cover_wall_dialog.dart';
import '../dialogs/play_on/play_on_dialog.dart';
import '../dialogs/add_to_playlist/show_add_to_playlist_dialog.dart';

import '../router/router_aware_flyout_controller.dart';
import 'utils/build_m3u8.dart';
import 'utils/collection_file_ids.dart';
import 'offers_mixes.dart';
import 'offline_selection.dart';
import '../dialogs/file_dialog/pick_files.dart';

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

/// The play, queue and roam items every menu of a playable item starts with,
/// on a large screen. [afterPlay] goes between the first two.
List<MenuFlyoutItemBase> _largeScreenPlayItems(
  BuildContext context, {
  required Future<void> Function() play,
  required Future<void> Function() queue,
  required Future<void> Function() roam,
  MenuFlyoutItemBase? afterPlay,
}) => [
  MenuFlyoutItem(
    leading: const Icon(Symbols.play_circle),
    text: Text(S.of(context).startPlaying),
    onPressed: () async {
      await _handleCollectionRequest(context, play);
    },
  ),
  ?afterPlay,
  MenuFlyoutItem(
    leading: const Icon(Symbols.playlist_add),
    text: Text(S.of(context).addToQueue),
    onPressed: () async {
      await _handleCollectionRequest(context, queue);
    },
  ),
  if (canStartRoaming(context))
    MenuFlyoutItem(
      leading: const Icon(Symbols.rocket),
      text: Text(S.of(context).startRoaming),
      onPressed: () async {
        await _handleCollectionRequest(context, roam);
      },
    ),
];

/// The same items as the buttons of the menu of a band or dock screen.
FlyoutContent _bandScreenPlayMenu(
  BuildContext context,
  bool isBand, {
  required Future<void> Function() play,
  required Future<void> Function() queue,
  required Future<void> Function() roam,
  CommandBarButton? afterPlay,
}) {
  final items = [
    CommandBarButton(
      icon: Tooltip(
        message: S.of(context).startPlaying,
        child: const Icon(Symbols.play_circle),
      ),
      onPressed: () async {
        await _handleCollectionRequest(context, play);
      },
    ),
    ?afterPlay,
    CommandBarButton(
      icon: Tooltip(
        message: S.of(context).addToQueue,
        child: const Icon(Symbols.playlist_add),
      ),
      onPressed: () async {
        await _handleCollectionRequest(context, queue);
      },
    ),
    if (canStartRoaming(context))
      CommandBarButton(
        icon: Tooltip(
          message: S.of(context).startRoaming,
          child: const Icon(Symbols.rocket),
        ),
        onPressed: () async {
          await _handleCollectionRequest(context, roam);
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
  Future<List<Mix>> Function() loadMixes = getAllMixes,
  Future<List<Playlist>> Function() loadPlaylists = getAllPlaylists,
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

  final withMixes = offersMixes(context);
  // A library that lists no playlists is not asked for them.
  final withPlaylists = canListPlaylists(context);
  late final List<Playlist> playlists;
  late final List<Mix>? mixes;
  try {
    playlists = withPlaylists ? await loadPlaylists() : const [];
    mixes = withMixes ? await loadMixes() : null;
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
      const [],
      playlists,
    ),
  );
}

/// The collection menu of a large screen. [mixes] is `null` when the open
/// library keeps no mixes, which leaves out Add to Mix.
MenuFlyout buildLargeScreenCollectionItemContextMenu(
  BuildContext context,
  CollectionType type,
  int id,
  String title,
  List<Mix>? mixes, [
  void Function()? refreshList,
  bool? readonly,
  List<int> fallbackFileIds = const [],
  List<Playlist> playlists = const [],
  Future<List<int>> Function(CollectionType type, int id) loadFileIds =
      collectionFileIds,
  Future<void> Function(int playlistId, int fileId) addFileToPlaylist =
      addItemToPlaylist,
  Future<List<(String, String)>> Function(CollectionType type, int id)
      loadQueries =
      buildQuery,
  Future<void> Function(int mixId, String operator, String parameter)
      addQueryToMix =
      addItemToMix,
]) {
  final operator = typeToOperator[type];
  final edit = typeToEdit[type];
  final remove = typeToRemove[type];
  final libraryState = context.read<LibraryOpenStateProvider>();
  final castLibraryId = libraryState.catalogLibraryId ?? libraryState.openRecord?.recordId;

  final isMix = type == CollectionType.mix;
  // Whether the collection resolves to queries, which the adds work from.
  final hasQueries = operator != null || isMix;

  // Adds what the collection is made of to a mix: the collection itself, or
  // each query of a mix.
  Future<void> addToMix(int mixId) async {
    for (final (operator, parameter) in await loadQueries(type, id)) {
      await addQueryToMix(mixId, operator, parameter);
    }
  }

  // Adds the songs of the collection that a playlist does not have yet, in
  // the collection's order, in a dialog that shows how far it is.
  Future<void> addToPlaylist(BuildContext host, int playlistId) =>
      showAddToPlaylistDialog(
        host,
        type,
        id,
        playlistId,
        loadFileIds: loadFileIds,
        addFileToPlaylist: addFileToPlaylist,
      );

  final List<MenuFlyoutItem> playlistItems = playlists
      .where((x) => type != CollectionType.playlist || x.id != id)
      .map((playlist) {
        return MenuFlyoutItem(
          leading: const Icon(Symbols.list_alt),
          text: Text(playlist.name),
          onPressed: () {
            final host = Navigator.of(context).context;
            Flyout.of(context).close();
            addToPlaylist(host, playlist.id);
          },
        );
      })
      .toList();

  // A mix is not added to itself.
  final List<MenuFlyoutItem>? mixItems = mixes
      ?.where((x) => !x.locked && !(isMix && x.id == id))
      .map((mix) {
        return MenuFlyoutItem(
          leading: const Icon(Symbols.magic_button),
          text: Text(mix.name),
          onPressed: () {
            final host = Navigator.of(context).context;
            Flyout.of(context).close();
            _handleCollectionRequest(host, () => addToMix(mix.id));
          },
        );
      })
      .toList();

  List<MenuFlyoutItemBase> items = _largeScreenPlayItems(
    context,
    play: () => startPlaying(type, id, fallbackFileIds),
    queue: () => addToQueue(type, id, fallbackFileIds),
    roam: () => startRoaming(context, type, id, fallbackFileIds),
    afterPlay:
        libraryState.openRecord != null &&
            !libraryState.isRemoteControl &&
            canPlayOnOtherDevice(context)
        ? MenuFlyoutItem(
            leading: const Icon(Symbols.cast),
            text: Text(S.of(context).playOn),
            onPressed: () async {
              final host = Navigator.of(context).context;
              Flyout.of(context).close();
              await _handleCollectionRequest(host, () async {
                await showCastOnDialog(
                  host,
                  libraryId: castLibraryId!,
                  cast: (candidate) => castCollectionWithSettings(
                    type,
                    id,
                    fallbackFileIds,
                    candidate.fingerprint,
                  ),
                );
              });
            },
          )
        : null,
  );

  if (edit != null && !context.read<LibraryOpenStateProvider>().isReadOnlyReplica) {
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
      !context.read<LibraryOpenStateProvider>().isReadOnlyReplica) {
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

  if (hasQueries &&
      canListPlaylists(context) &&
      !context.read<LibraryOpenStateProvider>().isReadOnlyReplica) {
    items.add(const MenuFlyoutSeparator());
    items.add(
      MenuFlyoutSubItem(
        leading: const Icon(Symbols.list_alt),
        text: Text(S.of(context).addToPlaylist),
        items: (context) => [
          MenuFlyoutItem(
            leading: const Icon(Symbols.add),
            text: Text(S.of(context).newPlaylist),
            onPressed: () async {
              final host = Navigator.of(context).context;
              Flyout.of(context).close();

              final playlist = await showCreateEditPlaylistDialog(
                host,
                title,
                playlistId: null,
              );

              if (playlist == null) return;

              if (host.mounted) {
                await addToPlaylist(host, playlist.id);
              }
            },
          ),
          if (playlistItems.isNotEmpty) const MenuFlyoutSeparator(),
          ...playlistItems,
        ],
      ),
    );
  }

  if (mixItems != null &&
      hasQueries &&
      !context.read<LibraryOpenStateProvider>().isReadOnlyReplica) {
    items.add(
      MenuFlyoutSubItem(
        leading: const Icon(Symbols.magic_button),
        text: Text(S.of(context).addToMix),
        items: (context) => [
          MenuFlyoutItem(
            leading: const Icon(Symbols.add),
            text: Text(S.of(context).newMix),
            onPressed: () async {
              final host = Navigator.of(context).context;
              Flyout.of(context).close();

              // The dialog takes one query, so the queries of a mix are added
              // to the new mix once it is made.
              final mix = await showCreateEditMixDialog(
                host,
                title,
                mixId: null,
                operator: operator == null ? null : (operator, id.toString()),
              );

              if (mix == null || operator != null) return;

              if (host.mounted) {
                await _handleCollectionRequest(host, () => addToMix(mix.id));
              }
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
          if (canExportPlaylistFile(context))
            MenuFlyoutItem(
              leading: const Icon(Symbols.list_alt),
              text: Text(S.of(context).exportM3u8),
              onPressed: () async {
                final host = Navigator.of(context).context;
                Flyout.of(context).close();

                final Directory appDocumentsDir =
                    await getApplicationDocumentsDirectory();

                if (!host.mounted) return;
                final String? path = await pickSaveFile(
                  host,
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

  if (supportsOfflineSelection(context)) {
    items.add(const MenuFlyoutSeparator());
    items.add(offlineSelectionMenu(context, type, id));
  }

  return MenuFlyout(items: items);
}

FlyoutContent buildBandScreenCollectionItemContextMenu(
  BuildContext context,
  CollectionType type,
  int id,
  bool isBand, {
  List<int> fallbackFileIds = const [],
}) {
  final libraryState = context.read<LibraryOpenStateProvider>();
  final castLibraryId = libraryState.catalogLibraryId ?? libraryState.openRecord?.recordId;
  return _bandScreenPlayMenu(
    context,
    isBand,
    play: () => startPlaying(type, id, fallbackFileIds),
    queue: () => addToQueue(type, id, fallbackFileIds),
    roam: () => startRoaming(context, type, id, fallbackFileIds),
    afterPlay:
        libraryState.openRecord != null &&
            !libraryState.isRemoteControl &&
            canPlayOnOtherDevice(context)
        ? CommandBarButton(
            icon: Tooltip(
              message: S.of(context).playOn,
              child: const Icon(Symbols.cast),
            ),
            onPressed: () async {
              final host = Navigator.of(context).context;
              await _handleCollectionRequest(host, () async {
                await showCastOnDialog(
                  host,
                  libraryId: castLibraryId!,
                  cast: (candidate) => castCollectionWithSettings(
                    type,
                    id,
                    fallbackFileIds,
                    candidate.fingerprint,
                  ),
                );
              });
            },
          )
        : null,
  );
}

/// Opens the menu of an item that is a list of [queries], as a release section
/// of an artist. It has the actions of a collection's menu that need no
/// collection to be named: start playing, add to queue and start roaming.
void openQueryListContextMenu(
  Offset localPosition,
  BuildContext context,
  GlobalKey contextAttachKey,
  RouterAwareFlyoutController contextController,
  QueryList queries,
) {
  final targetContext = contextAttachKey.currentContext;
  if (targetContext == null) return;
  final box = targetContext.findRenderObject() as RenderBox;
  final position = box.localToGlobal(
    localPosition,
    ancestor: Navigator.of(context).context.findRenderObject(),
  );

  final r = Provider.of<ResponsiveProvider>(context, listen: false);
  final isDock = r.smallerOrEqualTo(DeviceType.dock, false);
  final isBand = r.smallerOrEqualTo(DeviceType.band, false);

  contextController.showFlyout(
    position: position,
    builder: (context) => isDock || isBand
        ? buildBandScreenQueryListContextMenu(context, queries, isBand)
        : buildLargeScreenQueryListContextMenu(context, queries),
  );
}

MenuFlyout buildLargeScreenQueryListContextMenu(
  BuildContext context,
  QueryList queries,
) => MenuFlyout(
  items: _largeScreenPlayItems(
    context,
    play: () => startPlayingQueries(queries),
    queue: () => addQueriesToQueue(queries),
    roam: () => startRoamingQueries(context, queries),
  ),
);

FlyoutContent buildBandScreenQueryListContextMenu(
  BuildContext context,
  QueryList queries,
  bool isBand,
) => _bandScreenPlayMenu(
  context,
  isBand,
  play: () => startPlayingQueries(queries),
  queue: () => addQueriesToQueue(queries),
  roam: () => startRoamingQueries(context, queries),
);
