import '../api/request_failure.dart';
import '../dialogs/information/error.dart';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/l10n.dart';
import '../../utils/api/get_all_mixes.dart';
import '../../utils/api/add_item_to_mix.dart';
import '../../utils/api/get_all_playlists.dart';
import '../../utils/api/add_item_to_playlist.dart';
import '../../utils/api/get_parsed_media_file.dart';
import '../../utils/api/get_liked.dart';
import '../../utils/api/set_liked.dart';
import '../../utils/dialogs/mix/create_edit_mix.dart';
import '../../bindings/bindings.dart';
import '../../providers/responsive_providers.dart';
import '../../providers/library_open_state.dart';
import '../../providers/request_capabilities.dart';
import '../dialogs/play_on/play_on_dialog.dart';

import '../rune_log.dart';
import '../execute_middle_click_action.dart';
import '../api/remove_item_from_playlist.dart';
import '../router/navigation.dart';
import '../router/query_tracks_parameter.dart';
import '../router/router_aware_flyout_controller.dart';
import '../dialogs/playlist/create_edit_playlist.dart';
import '../playing_item.dart';
import 'like_item.dart';
import 'offers_mixes.dart';
import 'offline_selection.dart';

Future<void> _showRequestFailure(
  BuildContext context,
  RequestFailure error,
) async {
  if (!context.mounted) return;
  await showErrorDialog(
    context: context,
    title: S.of(context).unknownError,
    errorMessage: requestFailureMessage(S.of(context), error),
  );
}

Future<void> _runTrackAction(
  BuildContext context,
  Future<void> Function() action,
) async {
  final host = Navigator.of(context).context;
  try {
    await action();
  } on RequestFailure catch (error) {
    if (host.mounted) await _showRequestFailure(host, error);
  }
}

Future<void> _addToPlaylist(
  BuildContext context,
  int playlistId,
  int fileId,
) async {
  final host = Navigator.of(context).context;
  try {
    await addItemToPlaylist(playlistId, fileId);
  } on RequestFailure catch (error) {
    if (host.mounted) await _showRequestFailure(host, error);
  }
}

Future<void> _removeFromPlaylist(
  BuildContext context,
  int playlistId,
  String occurrenceId,
  void Function()? refreshList,
) async {
  final host = Navigator.of(context).context;
  try {
    await removeItemFromPlaylist(playlistId, occurrenceId);
    refreshList?.call();
  } on RequestFailure catch (error) {
    if (host.mounted) await _showRequestFailure(host, error);
  }
}

void openTrackItemContextMenu(
  Offset localPosition,
  BuildContext context,
  GlobalKey contextAttachKey,
  RouterAwareFlyoutController contextController,
  String? occurrenceId,
  int fileId,
  int? playlistId,
  void Function()? refreshList, {
  Future<List<Playlist>> Function() loadPlaylists = getAllPlaylists,
  Future<List<Mix>> Function() loadMixes = getAllMixes,
  Future<FetchParsedMediaFileResponse> Function(int fileId) loadMediaFile =
      getParsedMediaFile,
  Future<bool> Function(PlayingItem item) fetchLikedStatus = getLiked,
  Future<bool> Function(PlayingItem item, bool liked) setLikedStatus = setLiked,
}) async {
  final targetContext = contextAttachKey.currentContext;

  if (targetContext == null) return;
  final box = targetContext.findRenderObject() as RenderBox;
  final position = box.localToGlobal(
    localPosition,
    ancestor: Navigator.of(context).context.findRenderObject(),
  );
  if (fileId <= 0) {
    contextController.showFlyout(
      position: position,
      builder: (context) => MenuFlyout(
        items: [
          MenuFlyoutItem(
            text: Text(S.of(context).audioUnavailable),
            onPressed: null,
          ),
          if (!context.read<LibraryOpenStateProvider>().isReadOnlyReplica &&
              playlistId != null &&
              occurrenceId != null)
            MenuFlyoutItem(
              text: Text(S.of(context).removeFromPlaylist),
              onPressed: () async {
                await _removeFromPlaylist(
                  context,
                  playlistId,
                  occurrenceId,
                  refreshList,
                );
              },
            ),
        ],
      ),
    );
    return;
  }
  final withMixes = offersMixes(context);
  // A library that lists no playlists is not asked for them.
  final withPlaylists = canListPlaylists(context);
  late final List<Playlist> playlists;
  late final List<Mix>? mixes;
  late final FetchParsedMediaFileResponse parsedMediaFile;
  // Whether the track is liked, or null where the menu does not offer Like.
  bool? liked;
  try {
    // The three do not depend on each other, so they are asked together. Whether the track
    // is liked needs the track, and is asked after.
    (playlists, mixes, parsedMediaFile) = await (
      withPlaylists ? loadPlaylists() : Future.value(const <Playlist>[]),
      withMixes ? loadMixes() : Future<List<Mix>?>.value(null),
      loadMediaFile(fileId),
    ).wait;
    if (!context.mounted) return;
    final track = likeableTrack(parsedMediaFile.file);
    if (trackLikeOffered(context, track)) {
      liked = await readTrackLiked(context, track, fetchLikedStatus);
    }
  } on RequestFailure catch (error) {
    if (context.mounted) await _showRequestFailure(context, error);
    return;
  }

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
          playlistId: playlistId,
          occurrenceId: occurrenceId,
          refreshList: refreshList,
          liked: liked,
          setLikedStatus: setLikedStatus,
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
      occurrenceId,
      playlistId,
      refreshList,
      liked: liked,
      setLikedStatus: setLikedStatus,
    ),
  );
}

/// The track menu of a large screen. [mixes] is `null` when the open library
/// keeps no mixes, which leaves out Add to Mix. [liked] is whether the track is
/// liked, or null where Like is not offered.
Widget buildTrackItemContextMenu(
  BuildContext context,
  FetchParsedMediaFileResponse item,
  List<Playlist> playlists,
  List<Mix>? mixes,
  String? occurrenceId,
  int? playlistId,
  void Function()? refreshList, {
  bool? liked,
  Future<bool> Function(PlayingItem item, bool liked) setLikedStatus = setLiked,
}) {
  final List<MenuFlyoutItem> playlistItems = playlists.map((playlist) {
    return MenuFlyoutItem(
      leading: const Icon(Symbols.list_alt),
      text: Text(playlist.name),
      onPressed: () {
        _addToPlaylist(context, playlist.id, item.file.id);

        Flyout.of(context).close();
      },
    );
  }).toList();

  final List<MenuFlyoutItem>? mixItems = mixes?.where((x) => !x.locked).map((
    mixes,
  ) {
    return MenuFlyoutItem(
      leading: const Icon(Symbols.magic_button),
      text: Text(mixes.name),
      onPressed: () {
        final host = Navigator.of(context).context;
        Flyout.of(context).close();
        () async {
          try {
            await addItemToMix(mixes.id, "lib::track", item.file.id.toString());
          } on RequestFailure catch (error) {
            if (host.mounted) await _showRequestFailure(host, error);
          }
        }();
      },
    );
  }).toList();
  final libraryState = context.read<LibraryOpenStateProvider>();
  final castLibraryId = libraryState.catalogLibraryId ?? libraryState.openRecord?.recordId;

  return MenuFlyout(
    items: [
      MenuFlyoutItem(
        leading: const Icon(Symbols.play_circle),
        text: Text(S.of(context).startPlaying),
        onPressed: () async {
          await _runTrackAction(
            context,
            () => startPlaying(CollectionType.track, item.file.id, const []),
          );
        },
      ),
      if (libraryState.openRecord != null &&
          !libraryState.isRemoteControl &&
          canPlayOnOtherDevice(context))
        MenuFlyoutItem(
          leading: const Icon(Symbols.cast),
          text: Text(S.of(context).playOn),
          onPressed: () async {
            final host = Navigator.of(context).context;
            Flyout.of(context).close();
            await _runTrackAction(host, () async {
              await showCastOnDialog(
                host,
                libraryId: castLibraryId!,
                cast: (candidate) =>
                    castTrackWithSettings(item.file.id, candidate.fingerprint),
              );
            });
          },
        ),
      MenuFlyoutItem(
        leading: const Icon(Symbols.playlist_add),
        text: Text(S.of(context).addToQueue),
        onPressed: () async {
          await _runTrackAction(
            context,
            () => addToQueue(CollectionType.track, item.file.id, const []),
          );
        },
      ),
      if (canStartRoaming(context))
        MenuFlyoutItem(
          leading: const Icon(Symbols.rocket),
          text: Text(S.of(context).startRoaming),
          onPressed: () async {
            await _runTrackAction(
              context,
              () => startRoaming(
                context,
                CollectionType.track,
                item.file.id,
                const [],
              ),
            );
          },
        ),
      if (!context.read<LibraryOpenStateProvider>().isReadOnlyReplica &&
          playlistId != null &&
          occurrenceId != null)
        const MenuFlyoutSeparator(),
      if (!context.read<LibraryOpenStateProvider>().isReadOnlyReplica &&
          playlistId != null &&
          occurrenceId != null)
        MenuFlyoutItem(
          leading: const Icon(Symbols.delete),
          text: Text(S.of(context).removeFromPlaylist),
          onPressed: () async {
            try {
              await _removeFromPlaylist(
                context,
                playlistId,
                occurrenceId,
                refreshList,
              );
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
            $goTo(
              '/artists/detail',
              arguments: QueryTracksParameter(
                item.artists[0].id,
                item.artists[0].name,
              ),
            ),
          },
        ),
      if (item.artists.length > 1)
        MenuFlyoutSubItem(
          leading: const Icon(Symbols.face),
          text: Text(S.of(context).goToArtist),
          items: (context) => item.artists
              .map(
                (x) => MenuFlyoutItem(
                  leading: const Icon(Symbols.face),
                  text: Text(x.name),
                  onPressed: () => {
                    $goTo(
                      '/artists/detail',
                      arguments: QueryTracksParameter(x.id, x.name),
                    ),
                  },
                ),
              )
              .toList(),
        ),
      MenuFlyoutItem(
        leading: const Icon(Symbols.album),
        text: Text(S.of(context).goToAlbum),
        onPressed: () => {
          $goTo(
            '/albums/detail',
            arguments: QueryTracksParameter(item.album.id, item.album.name),
          ),
        },
      ),
      if (!context.read<LibraryOpenStateProvider>().isReadOnlyReplica)
        const MenuFlyoutSeparator(),
      if (liked != null)
        likeMenuItem(
          context,
          liked: liked,
          onPressed: () async {
            await _runTrackAction(
              context,
              () => setLikedStatus(likeableTrack(item.file), !liked),
            );
          },
        ),
      if (!context.read<LibraryOpenStateProvider>().isReadOnlyReplica)
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
                  item.file.title,
                  playlistId: null,
                );

                if (playlist == null) return;

                if (host.mounted) {
                  await _addToPlaylist(host, playlist.id, item.file.id);
                }
              },
            ),
            if (playlistItems.isNotEmpty) const MenuFlyoutSeparator(),
            ...playlistItems,
          ],
        ),
      if (mixItems != null &&
          !context.read<LibraryOpenStateProvider>().isReadOnlyReplica)
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

                final playlist = await showCreateEditMixDialog(
                  host,
                  item.file.title,
                  mixId: null,
                );

                if (playlist == null) return;

                if (!host.mounted) return;
                await _runTrackAction(
                  host,
                  () => addItemToMix(
                    playlist.id,
                    "lib::track",
                    item.file.id.toString(),
                  ),
                );
              },
            ),
            if (mixItems.isNotEmpty) const MenuFlyoutSeparator(),
            ...mixItems,
          ],
        ),
      if (supportsOfflineSelection(context)) const MenuFlyoutSeparator(),
      if (supportsOfflineSelection(context))
        offlineSelectionMenu(context, CollectionType.track, item.file.id),
    ],
  );
}

FlyoutContent buildBandScreenTrackItemContextMenu(
  BuildContext context,
  FetchParsedMediaFileResponse item,
  bool isBand, {
  int? playlistId,
  String? occurrenceId,
  void Function()? refreshList,
  bool? liked,
  Future<bool> Function(PlayingItem item, bool liked) setLikedStatus = setLiked,
}) {
  final libraryState = context.read<LibraryOpenStateProvider>();
  final castLibraryId = libraryState.catalogLibraryId ?? libraryState.openRecord?.recordId;
  List<CommandBarButton> items = [
    CommandBarButton(
      icon: Tooltip(
        message: S.of(context).startPlaying,
        child: const Icon(Symbols.play_circle),
      ),
      onPressed: () async {
        await _runTrackAction(
          context,
          () => startPlaying(CollectionType.track, item.file.id, const []),
        );
      },
    ),
    if (libraryState.openRecord != null &&
        !libraryState.isRemoteControl &&
        canPlayOnOtherDevice(context))
      CommandBarButton(
        icon: Tooltip(
          message: S.of(context).playOn,
          child: const Icon(Symbols.cast),
        ),
        onPressed: () async {
          await _runTrackAction(context, () async {
            await showCastOnDialog(
              context,
              libraryId: castLibraryId!,
              cast: (candidate) =>
                  castTrackWithSettings(item.file.id, candidate.fingerprint),
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
        await _runTrackAction(
          context,
          () => addToQueue(CollectionType.track, item.file.id, const []),
        );
      },
    ),
    if (canStartRoaming(context))
      CommandBarButton(
        icon: Tooltip(
          message: S.of(context).startRoaming,
          child: const Icon(Symbols.rocket),
        ),
        onPressed: () async {
          await _runTrackAction(
            context,
            () => startRoaming(
              context,
              CollectionType.track,
              item.file.id,
              const [],
            ),
          );
        },
      ),
    if (!context.read<LibraryOpenStateProvider>().isReadOnlyReplica &&
        playlistId != null &&
        occurrenceId != null)
      CommandBarButton(
        icon: Tooltip(
          message: S.of(context).removeFromPlaylist,
          child: const Icon(Symbols.delete),
        ),
        onPressed: () async {
          try {
            await _removeFromPlaylist(
              context,
              playlistId,
              occurrenceId,
              refreshList,
            );
          } catch (error) {
            error$(error.toString());
          }
        },
      ),
    if (liked != null)
      likeBandButton(
        context,
        liked: liked,
        onPressed: () async {
          final host = Navigator.of(context).context;
          Flyout.of(context).close();
          await _runTrackAction(
            host,
            () => setLikedStatus(likeableTrack(item.file), !liked),
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
