import 'dart:math';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_boring_avatars/flutter_boring_avatars.dart';
import 'package:go_router/go_router.dart';

import '../../../utils/router_extra.dart';
import '../../../utils/context_menu/collection_item_context_menu.dart';
import '../../../widgets/flip_grid.dart';
import '../../../widgets/context_menu_wrapper.dart';

abstract class SearchCard extends StatelessWidget {
  final int index;
  final FlyoutController contextController = FlyoutController();
  final GlobalKey contextAttachKey = GlobalKey();

  SearchCard({super.key, required this.index});

  int getItemId();
  String getItemTitle();
  Widget buildLeadingWidget(double size);
  void onPressed(BuildContext context);
  void onContextMenu(BuildContext context, Offset position);

  @override
  Widget build(BuildContext context) {
    return ContextMenuWrapper(
      contextAttachKey: contextAttachKey,
      contextController: contextController,
      onContextMenu: (position) => onContextMenu(context, position),
      child: Button(
        style: const ButtonStyle(
          padding: WidgetStatePropertyAll(EdgeInsets.all(0)),
        ),
        onPressed: () => onPressed(context),
        child: ClipRRect(
          borderRadius: BorderRadius.circular(3),
          child: LayoutBuilder(
            builder: (context, constraints) {
              final size = min(constraints.maxWidth, constraints.maxHeight);
              return Row(
                children: [
                  buildLeadingWidget(size),
                  Expanded(
                    child: Padding(
                      padding: const EdgeInsets.all(8),
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        mainAxisAlignment: MainAxisAlignment.center,
                        children: [
                          Text(
                            getItemTitle(),
                            overflow: TextOverflow.ellipsis,
                          ),
                        ],
                      ),
                    ),
                  ),
                ],
              );
            },
          ),
        ),
      ),
    );
  }
}

abstract class CollectionSearchCard<T> extends SearchCard {
  final T item;
  final String routePrefix;
  final BoringAvatarType emptyTileType;

  CollectionSearchCard({
    super.key,
    required super.index,
    required this.item,
    required this.routePrefix,
    required this.emptyTileType,
  });

  @override
  int getItemId();

  @override
  String getItemTitle();

  @override
  Widget buildLeadingWidget(double size) {
    return SizedBox(
      width: size,
      height: size,
      child: FlipCoverGrid(
        numbers: getCoverIds(),
        id: getItemTitle(),
        emptyTileType: BoringAvatarType.bauhaus,
      ),
    );
  }

  List<int> getCoverIds();

  @override
  void onPressed(BuildContext context) {
    context.replace('/$routePrefix/${getItemId()}',
        extra: QueryTracksExtra(getItemTitle()));
  }

  @override
  void onContextMenu(BuildContext context, Offset position) {
    openCollectionItemContextMenu(position, context, contextAttachKey,
        contextController, routePrefix, getItemId());
  }
}
