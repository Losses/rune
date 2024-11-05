import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_boring_avatars/flutter_boring_avatars.dart';

import '../../utils/router/navigation.dart';
import '../../utils/router/router_name.dart';
import '../../utils/router/query_tracks_parameter.dart';
import '../../utils/context_menu/collection_item_context_menu.dart';
import '../../widgets/ax_pressure.dart';
import '../../widgets/tile/flip_tile.dart';
import '../../widgets/context_menu_wrapper.dart';
import '../../widgets/start_screen/utils/internal_collection.dart';

import '../../messages/collection.pb.dart';

class CollectionItem extends StatefulWidget {
  final InternalCollection collection;
  final CollectionType collectionType;
  final VoidCallback refreshList;

  const CollectionItem({
    super.key,
    required this.collection,
    required this.collectionType,
    required this.refreshList,
  });

  @override
  State<CollectionItem> createState() => _CollectionItemState();
}

class _CollectionItemState extends State<CollectionItem> {
  final contextController = FlyoutController();

  final contextAttachKey = GlobalKey();

  @override
  dispose() {
    super.dispose();
    contextController.dispose();
  }

  List<String> filterDuplicates(List<String> input) {
    Set<String> seen = {};
    int blankCount = 0;
    List<String> result = [];

    for (var item in input) {
      if (item.trim().isEmpty) {
        if (blankCount < 5) {
          result.add(item);
          blankCount++;
        }
      } else if (!seen.contains(item)) {
        seen.add(item);
        result.add(item);
      }
    }

    return result;
  }

  @override
  Widget build(BuildContext context) {
    return AxPressure(
      child: ContextMenuWrapper(
        contextAttachKey: contextAttachKey,
        contextController: contextController,
        onContextMenu: (position) {
          openCollectionItemContextMenu(
            position,
            context,
            contextAttachKey,
            contextController,
            widget.collectionType,
            widget.collection.id,
            widget.refreshList,
            widget.collection.readonly,
          );
        },
        child: FlipTile(
          name: widget.collection.name,
          paths:
              filterDuplicates(widget.collection.coverArtMap.values.toList()),
          emptyTileType: BoringAvatarType.bauhaus,
          onPressed: () {
            $push(
              '/${collectionTypeToRouterName(widget.collectionType)}/detail',
              arguments: QueryTracksParameter(
                widget.collection.id,
                widget.collection.name,
              ),
            );
          },
        ),
      ),
    );
  }
}
