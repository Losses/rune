import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_boring_avatars/flutter_boring_avatars.dart';

import '../../../utils/execute_middle_click_action.dart';
import '../../../utils/router/navigation.dart';
import '../../../utils/router/router_name.dart';
import '../../../utils/router/query_tracks_parameter.dart';
import '../../../utils/context_menu/collection_item_context_menu.dart';
import '../../../widgets/tile/flip_cover_grid.dart';
import '../../../widgets/start_screen/utils/internal_collection.dart';
import '../../../screens/search/widgets/search_card.dart';
import '../../../messages/collection.pb.dart';

class CollectionSearchItem extends SearchCard {
  final InternalCollection item;
  final CollectionType collectionType;
  final BoringAvatarType emptyTileType = BoringAvatarType.marble;

  CollectionSearchItem({
    super.key,
    super.index = 0,
    required this.item,
    required this.collectionType,
  });

  @override
  void onPressed(BuildContext context) {
    $replace(
      '/${collectionTypeToRouterName(collectionType)}/detail',
      arguments: QueryTracksParameter(getItemId(), getItemTitle()),
    );
  }

  @override
  void onContextMenu(BuildContext context, Offset position) {
    openCollectionItemContextMenu(
      position,
      context,
      contextAttachKey,
      contextController,
      collectionType,
      getItemId(),
    );
  }

  @override
  void onMiddleClick(BuildContext context, Offset position) {
    executeMiddleClickAction(
      context,
      collectionType,
      item.id,
    );
  }

  @override
  int getItemId() => item.id;

  @override
  String getItemTitle() => item.name;

  @override
  Widget buildLeadingWidget(double size) {
    return SizedBox(
      width: size,
      height: size,
      child: FlipCoverGrid(
        id: getItemTitle(),
        paths: item.coverArtMap.values.toList(),
        emptyTileType: BoringAvatarType.bauhaus,
      ),
    );
  }
}
