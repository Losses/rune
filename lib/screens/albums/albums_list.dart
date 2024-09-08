import 'package:fluent_ui/fluent_ui.dart';
import 'package:go_router/go_router.dart';
import 'package:flutter_boring_avatars/flutter_boring_avatars.dart';
import 'package:player/widgets/context_menu_wrapper.dart';

import '../../../utils/router_extra.dart';
import '../../../utils/context_menu/collection_item_context_menu.dart';
import '../../../widgets/flip_tile.dart';
import '../../../widgets/grouped_list_base.dart';
import '../../../widgets/start_screen/start_screen.dart';
import '../../../messages/album.pb.dart';

class AlbumsListView extends GroupedListBase<Album, AlbumsGroupSummary> {
  const AlbumsListView({super.key});

  @override
  AlbumsListViewState createState() => AlbumsListViewState();
}

class AlbumsListViewState
    extends GroupedListBaseState<Album, AlbumsGroupSummary> {
  @override
  Future<List<Group<Album>>> fetchSummary() async {
    final fetchAlbumsGroupSummary = FetchAlbumsGroupSummaryRequest();
    fetchAlbumsGroupSummary.sendSignalToRust(); // GENERATED

    final rustSignal = await AlbumGroupSummaryResponse.rustSignalStream.first;
    final albumGroupList = rustSignal.message;

    return albumGroupList.albumsGroups.map((summary) {
      return Group<Album>(
        groupTitle: summary.groupTitle,
        items: [], // Initially empty, will be filled in fetchPage
      );
    }).toList();
  }

  @override
  Future<List<Group<Album>>> fetchGroups(List<String> groupTitles) async {
    final fetchAlbumsGroupsRequest = FetchAlbumsGroupsRequest()
      ..groupTitles.addAll(groupTitles);
    fetchAlbumsGroupsRequest.sendSignalToRust(); // GENERATED

    final rustSignal = await AlbumsGroups.rustSignalStream.first;
    final albumsGroups = rustSignal.message.groups;

    return albumsGroups.map((group) {
      return Group<Album>(
        groupTitle: group.groupTitle,
        items: group.albums,
      );
    }).toList();
  }

  @override
  Widget itemBuilder(BuildContext context, Album item) {
    return AlbumItem(album: item);
  }
}

class AlbumItem extends StatelessWidget {
  final Album album;

  AlbumItem({
    super.key,
    required this.album,
  });

  final contextController = FlyoutController();
  final contextAttachKey = GlobalKey();

  @override
  Widget build(BuildContext context) {
    return ContextMenuWrapper(
      contextAttachKey: contextAttachKey,
      contextController: contextController,
      onContextMenu: (position) {
        openCollectionItemContextMenu(position, context, contextAttachKey,
            contextController, 'album', album.id);
      },
      child: FlipTile(
        name: album.name,
        coverIds: album.coverIds,
        emptyTileType: BoringAvatarType.marble,
        onPressed: () {
          context.push('/albums/${album.id}',
              extra: QueryTracksExtra(album.name));
        },
      ),
    );
  }
}
