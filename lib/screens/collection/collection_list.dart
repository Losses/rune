import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:go_router/go_router.dart';
import 'package:flutter_boring_avatars/flutter_boring_avatars.dart';
import 'package:infinite_scroll_pagination/infinite_scroll_pagination.dart';

import '../../utils/router_extra.dart';
import '../../utils/api/fetch_collection_groups.dart';
import '../../utils/api/fetch_collection_group_summary.dart';
import '../../utils/context_menu/collection_item_context_menu.dart';
import '../../widgets/tile/flip_tile.dart';
import '../../widgets/tile/cover_art_manager.dart';
import '../../widgets/context_menu_wrapper.dart';
import '../../widgets/start_screen/start_screen.dart';

import '../../messages/collection.pb.dart';

class CollectionListView extends StatefulWidget {
  final CollectionType collectionType;

  const CollectionListView({super.key, required this.collectionType});

  @override
  CollectionListViewState createState() => CollectionListViewState();
}

class CollectionListViewState extends State<CollectionListView> {
  static const _pageSize = 3;

  final PagingController<int, Group<InternalCollection>> pagingController =
      PagingController(firstPageKey: 0);
  final coverArtManager = CoverArtManager();

  Future<List<Group<InternalCollection>>> fetchSummary() async {
    final groups = await fetchCollectionGroupSummary(widget.collectionType);

    return groups.map((summary) {
      return Group<InternalCollection>(
        groupTitle: summary.groupTitle,
        items: [], // Initially empty, will be filled in fetchPage
      );
    }).toList();
  }

  Future<void> fetchPage(
    PagingController<int, Group<InternalCollection>> controller,
    int cursor,
  ) async {
    try {
      final summaries = await fetchSummary();

      final startIndex = cursor * _pageSize;
      final endIndex = (cursor + 1) * _pageSize;

      if (startIndex >= summaries.length) {
        controller.appendLastPage([]);
        return;
      }

      final currentPageSummaries = summaries.sublist(
        startIndex,
        endIndex > summaries.length ? summaries.length : endIndex,
      );

      final groupTitles =
          currentPageSummaries.map((summary) => summary.groupTitle).toList();

      final groups = await fetchGroups(groupTitles);

      for (final group in groups) {
        for (final collection in group.items) {
          // [1] We batch cache data here, find [2] for where to use
          await coverArtManager.queryCoverArts(collection.queries);
        }
      }

      final isLastPage = endIndex >= summaries.length;
      if (isLastPage) {
        controller.appendLastPage(groups);
      } else {
        controller.appendPage(groups, cursor + 1);
      }
    } catch (error) {
      controller.error = error;
    }
  }

  bool userGenerated() {
    return widget.collectionType == CollectionType.Playlist ||
        widget.collectionType == CollectionType.Mix;
  }

  Future<List<Group<InternalCollection>>> fetchGroups(
      List<String> groupTitles) async {
    final groups =
        await fetchCollectionGroups(widget.collectionType, groupTitles);

    return groups.map((group) {
      return Group<InternalCollection>(
        groupTitle: group.groupTitle,
        items: group.collections
            .map(InternalCollection.fromRawCollection)
            .toList(),
      );
    }).toList();
  }

  Widget itemBuilder(BuildContext context, InternalCollection item) {
    return CollectionItem(
      collection: item,
      collectionType: widget.collectionType,
      // [2] We use it here
      coverArtIds: coverArtManager.getResult(item.queries),
    );
  }

  @override
  Widget build(BuildContext context) {
    return ChangeNotifierProvider<CoverArtManager>.value(
      value: coverArtManager,
      child: StartScreen(
        fetchSummary: fetchSummary,
        fetchPage: fetchPage,
        itemBuilder: itemBuilder,
        pagingController: pagingController,
        userGenerated: userGenerated(),
      ),
    );
  }

  @override
  void dispose() {
    super.dispose();
    pagingController.dispose();
    coverArtManager.dispose();
  }
}

final Map<CollectionType, String> routerName = {
  CollectionType.Album: 'albums',
  CollectionType.Artist: 'artists',
  CollectionType.Playlist: 'playlists',
  CollectionType.Mix: 'mixes',
};

class CollectionItem extends StatelessWidget {
  final InternalCollection collection;
  final CollectionType collectionType;
  final List<int>? coverArtIds;

  CollectionItem({
    super.key,
    required this.collection,
    required this.collectionType,
    required this.coverArtIds,
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
        coverArtIds: coverArtIds,
        emptyTileType: BoringAvatarType.bauhaus,
        onPressed: () {
          context.push('/${routerName[collectionType]}/${collection.id}',
              extra: QueryTracksExtra(collection.name));
        },
      ),
    );
  }
}
