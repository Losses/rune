import 'package:fluent_ui/fluent_ui.dart';
import 'package:go_router/go_router.dart';
import 'package:flutter_boring_avatars/flutter_boring_avatars.dart';

import '../../../main.dart';
import '../../../utils/router_extra.dart';
import '../../../utils/context_menu/collection_item_context_menu.dart';
import '../../../widgets/flip_tile.dart';
import '../../../widgets/grouped_list_base.dart';
import '../../../widgets/start_screen/start_screen.dart';
import '../../../messages/artist.pb.dart';

class ArtistsListView extends GroupedListBase<Artist, ArtistsGroupSummary> {
  const ArtistsListView({super.key});

  @override
  ArtistsListViewState createState() => ArtistsListViewState();
}

class ArtistsListViewState
    extends GroupedListBaseState<Artist, ArtistsGroupSummary> {
  @override
  Future<List<Group<Artist>>> fetchSummary() async {
    final fetchArtistsGroupSummary = FetchArtistsGroupSummaryRequest();
    fetchArtistsGroupSummary.sendSignalToRust(); // GENERATED

    final rustSignal = await ArtistGroupSummaryResponse.rustSignalStream.first;
    final artistGroupList = rustSignal.message;

    return artistGroupList.artistsGroups.map((summary) {
      return Group<Artist>(
        groupTitle: summary.groupTitle,
        items: [], // Initially empty, will be filled in fetchPage
      );
    }).toList();
  }

  @override
  Future<List<Group<Artist>>> fetchGroups(List<String> groupTitles) async {
    final fetchArtistsGroupsRequest = FetchArtistsGroupsRequest()
      ..groupTitles.addAll(groupTitles);
    fetchArtistsGroupsRequest.sendSignalToRust(); // GENERATED

    final rustSignal = await ArtistsGroups.rustSignalStream.first;
    final artistsGroups = rustSignal.message.groups;

    return artistsGroups.map((group) {
      return Group<Artist>(
        groupTitle: group.groupTitle,
        items: group.artists,
      );
    }).toList();
  }

  @override
  Widget itemBuilder(BuildContext context, Artist item) {
    return ArtistItem(artist: item);
  }
}

class ArtistItem extends StatelessWidget {
  final Artist artist;

  ArtistItem({
    super.key,
    required this.artist,
  });

  final contextController = FlyoutController();
  final contextAttachKey = GlobalKey();

  void openContextMenu(Offset localPosition, BuildContext context) async {
    final targetContext = contextAttachKey.currentContext;

    if (targetContext == null) return;
    final box = targetContext.findRenderObject() as RenderBox;
    final position = box.localToGlobal(
      localPosition,
      ancestor: Navigator.of(context).context.findRenderObject(),
    );

    contextController.showFlyout(
      position: position,
      builder: (context) =>
          buildCollectionItemContextMenu(context, 'artist', artist.id),
    );
  }

  @override
  Widget build(BuildContext context) {
    return GestureDetector(
        onSecondaryTapUp: isDesktop
            ? (d) {
                openContextMenu(d.localPosition, context);
              }
            : null,
        onLongPressEnd: isDesktop
            ? null
            : (d) {
                openContextMenu(d.localPosition, context);
              },
        child: FlyoutTarget(
            key: contextAttachKey,
            controller: contextController,
            child: FlipTile(
              name: artist.name,
              coverIds: artist.coverIds,
              emptyTileType: BoringAvatarsType.bauhaus,
              onPressed: () => {
                context.push('/artists/${artist.id}',
                    extra: QueryTracksExtra(artist.name))
              },
            )));
  }
}
