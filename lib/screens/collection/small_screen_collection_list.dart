import 'package:fluent_ui/fluent_ui.dart';
import 'package:go_router/go_router.dart';
import 'package:flutter_boring_avatars/flutter_boring_avatars.dart';

import '../../utils/router_name.dart';
import '../../utils/router_extra.dart';
import '../../utils/context_menu/collection_item_context_menu.dart';
import '../../screens/collection/utils/fetch_page.dart';
import '../../screens/collection/utils/fetch_groups.dart';
import '../../screens/collection/utils/fetch_summary.dart';
import '../../screens/collection/utils/is_user_generated.dart';
import '../../widgets/tile/flip_tile.dart';
import '../../widgets/context_menu_wrapper.dart';
import '../../widgets/turntile/turntile_screen.dart';
import '../../widgets/start_screen/utils/group.dart';
import '../../widgets/start_screen/utils/internal_collection.dart';

import '../../messages/collection.pb.dart';

class SmallScreenCollectionListView extends StatefulWidget {
  final CollectionType collectionType;

  const SmallScreenCollectionListView(
      {super.key, required this.collectionType});

  @override
  SmallScreenCollectionListViewState createState() =>
      SmallScreenCollectionListViewState();
}

class SmallScreenCollectionListViewState
    extends State<SmallScreenCollectionListView> {
  static const _pageSize = 3;

  Future<List<Group<InternalCollection>>> fetchSummary() {
    return fetchCollectionPageSummary(widget.collectionType);
  }

  Future<(List<Group<InternalCollection>>, bool)> fetchPage(
    int cursor,
  ) async {
    return fetchCollectionPagePage(widget.collectionType, _pageSize, cursor);
  }

  Future<List<Group<InternalCollection>>> fetchGroups(
    List<String> groupTitles,
  ) {
    return fetchCollectionPageGroups(widget.collectionType, groupTitles);
  }

  Widget itemBuilder(
    BuildContext context,
    InternalCollection item,
    VoidCallback refreshList,
  ) {
    return CollectionItem(
      collection: item,
      collectionType: widget.collectionType,
      refreshList: refreshList,
    );
  }

  @override
  Widget build(BuildContext context) {
    return TurntileScreen(
      fetchSummary: fetchSummary,
      fetchPage: fetchPage,
      itemBuilder: itemBuilder,
      userGenerated: userGenerated(widget.collectionType),
    );
  }
}

class CollectionItem extends StatelessWidget {
  final InternalCollection collection;
  final CollectionType collectionType;
  final VoidCallback refreshList;

  CollectionItem({
    super.key,
    required this.collection,
    required this.collectionType,
    required this.refreshList,
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
          refreshList,
        );
      },
      child: FlipTile(
        name: collection.name,
        paths: collection.coverArtMap.values.toList(),
        emptyTileType: BoringAvatarType.bauhaus,
        onPressed: () {
          context.push(
            '/${collectionTypeToRouterName(collectionType)}/${collection.id}',
            extra: QueryTracksExtra(collection.name),
          );
        },
      ),
    );
  }
}
