import 'package:fluent_ui/fluent_ui.dart';
import 'package:go_router/go_router.dart';
import 'package:flutter_boring_avatars/flutter_boring_avatars.dart';

import '../../../main.dart';
import '../../../utils/router_extra.dart';
import '../../../utils/context_menu/collection_item_context_menu.dart';
import '../../../widgets/flip_tile.dart';
import '../../../widgets/grouped_list_base.dart';
import '../../../widgets/start_screen/start_screen.dart';
import '../../../messages/playlist.pb.dart';

class PlaylistsListView
    extends GroupedListBase<Playlist, PlaylistsGroupSummary> {
  const PlaylistsListView({super.key});

  @override
  PlaylistsListViewState createState() => PlaylistsListViewState();
}

class PlaylistsListViewState
    extends GroupedListBaseState<Playlist, PlaylistsGroupSummary> {
  @override
  Future<List<Group<Playlist>>> fetchSummary() async {
    final fetchPlaylistsGroupSummary = FetchPlaylistsGroupSummaryRequest();
    fetchPlaylistsGroupSummary.sendSignalToRust(); // GENERATED

    final rustSignal =
        await PlaylistGroupSummaryResponse.rustSignalStream.first;
    final playlistGroupList = rustSignal.message;

    return playlistGroupList.playlistsGroups.map((summary) {
      return Group<Playlist>(
        groupTitle: summary.groupTitle,
        items: [], // Initially empty, will be filled in fetchPage
      );
    }).toList();
  }

  @override
  Future<List<Group<Playlist>>> fetchGroups(List<String> groupTitles) async {
    final fetchPlaylistsGroupsRequest = FetchPlaylistsGroupsRequest()
      ..groupTitles.addAll(groupTitles);
    fetchPlaylistsGroupsRequest.sendSignalToRust(); // GENERATED

    final rustSignal = await PlaylistsGroups.rustSignalStream.first;
    final playlistsGroups = rustSignal.message.groups;

    return playlistsGroups.map((group) {
      return Group<Playlist>(
        groupTitle: group.groupTitle,
        items: group.playlists,
      );
    }).toList();
  }

  @override
  Widget itemBuilder(BuildContext context, Playlist item) {
    return PlaylistItem(playlist: item);
  }
}

class PlaylistItem extends StatelessWidget {
  final Playlist playlist;

  PlaylistItem({
    super.key,
    required this.playlist,
  });

  final contextController = FlyoutController();
  final contextAttachKey = GlobalKey();

  @override
  Widget build(BuildContext context) {
    return GestureDetector(
        onSecondaryTapUp: isDesktop
            ? (d) {
                openCollectionItemContextMenu(d.localPosition, context,
                    contextAttachKey, contextController, 'playlist', playlist.id);
              }
            : null,
        onLongPressEnd: isDesktop
            ? null
            : (d) {
                openCollectionItemContextMenu(d.localPosition, context,
                    contextAttachKey, contextController, 'playlist', playlist.id);
              },
        child: FlyoutTarget(
            key: contextAttachKey,
            controller: contextController,
            child: FlipTile(
              name: playlist.name,
              coverIds: playlist.coverIds,
              emptyTileType: BoringAvatarsType.bauhaus,
              onPressed: () => {
                context.push('/playlists/${playlist.id}',
                    extra: QueryTracksExtra(playlist.name))
              },
            )));
  }
}
