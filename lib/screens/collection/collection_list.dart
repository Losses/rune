import 'package:fluent_ui/fluent_ui.dart';
import 'package:go_router/go_router.dart';
import 'package:flutter_boring_avatars/flutter_boring_avatars.dart';

import '../../utils/query_list.dart';
import '../../utils/router_extra.dart';
import '../../utils/build_collection_query.dart';
import '../../utils/api/fetch_collection_groups.dart';
import '../../utils/api/fetch_collection_group_summary.dart';
import '../../utils/context_menu/collection_item_context_menu.dart';
import '../../widgets/grouped_list_base.dart';
import '../../widgets/tile/flip_tile.dart';
import '../../widgets/context_menu_wrapper.dart';
import '../../widgets/start_screen/start_screen.dart';

import '../../messages/collection.pb.dart';

class CollectionListView
    extends GroupedListBase<Collection, CollectionGroupSummary> {
  const CollectionListView({super.key, required super.collectionType});

  @override
  CollectionListViewState createState() => CollectionListViewState();
}

class CollectionListViewState
    extends GroupedListBaseState<Collection, CollectionGroupSummary> {
  @override
  Future<List<Group<Collection>>> fetchSummary() async {
    final groups = await fetchCollectionGroupSummary(widget.collectionType);

    return groups.map((summary) {
      return Group<Collection>(
        groupTitle: summary.groupTitle,
        items: [], // Initially empty, will be filled in fetchPage
      );
    }).toList();
  }

  @override
  bool userGenerated() {
    return false;
  }

  @override
  Future<List<Group<Collection>>> fetchGroups(List<String> groupTitles) async {
    final groups =
        await fetchCollectionGroups(widget.collectionType, groupTitles);

    return groups.map((group) {
      return Group<Collection>(
        groupTitle: group.groupTitle,
        items: group.collections,
      );
    }).toList();
  }

  @override
  Widget itemBuilder(BuildContext context, Collection item) {
    return CollectionItem(
      collection: item,
      collectionType: widget.collectionType,
    );
  }
}

final Map<CollectionType, String> routerName = {
  CollectionType.Album: 'albums',
  CollectionType.Artist: 'artists',
  CollectionType.Playlist: 'playlists',
  CollectionType.Mix: 'mixes',
};

class CollectionItem extends StatelessWidget {
  final Collection collection;
  final CollectionType collectionType;

  CollectionItem({
    super.key,
    required this.collection,
    required this.collectionType,
  });

  final contextController = FlyoutController();
  final contextAttachKey = GlobalKey();

  @override
  Widget build(BuildContext context) {
    return ContextMenuWrapper(
      contextAttachKey: contextAttachKey,
      contextController: contextController,
      onContextMenu: (position) {
        openCollectionItemContextMenu(
          position,
          context,
          contextAttachKey,
          contextController,
          collectionType,
          collection.id,
        );
      },
      child: FlipTile(
        name: collection.name,
        queries: QueryList(buildCollectionQuery(collectionType, collection.id)),
        emptyTileType: BoringAvatarType.bauhaus,
        onPressed: () {
          context.push('/${routerName[collectionType]}/${collection.id}',
              extra: QueryTracksExtra(collection.name));
        },
      ),
    );
  }
}
