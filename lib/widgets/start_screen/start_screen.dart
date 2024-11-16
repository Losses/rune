import 'dart:async';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/material_symbols_icons.dart';
import 'package:very_good_infinite_list/very_good_infinite_list.dart';

import '../../utils/dialogs/mix/mix_studio.dart';
import '../../utils/dialogs/show_group_list_dialog.dart';
import '../../utils/dialogs/playlist/create_edit_playlist.dart';
import '../../config/animation.dart';
import '../../widgets/no_items.dart';
import '../../widgets/context_menu_wrapper.dart';
import '../../widgets/start_screen/constants/default_gap_size.dart';
import '../../screens/collection/utils/is_user_generated.dart';
import '../../screens/collection/utils/collection_item_builder.dart';
import '../../screens/collection/utils/collection_data_provider.dart';
import '../../messages/all.dart';
import '../../utils/l10n.dart';

import '../infinite_list_loading.dart';
import '../smooth_horizontal_scroll.dart';
import '../navigation_bar/page_content_frame.dart';

import 'utils/group.dart';
import 'utils/internal_collection.dart';
import 'providers/start_screen_layout_manager.dart';

import 'start_group.dart';
import 'start_group_implementation.dart';

class StartScreen extends StatelessWidget {
  const StartScreen({super.key});

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        return StartScreenImplementation(
          constraints: constraints,
        );
      },
    );
  }
}

class StartScreenImplementation extends StatefulWidget {
  final BoxConstraints constraints;

  const StartScreenImplementation({
    super.key,
    required this.constraints,
  });

  @override
  StartScreenImplementationState createState() =>
      StartScreenImplementationState();
}

class StartScreenImplementationState extends State<StartScreenImplementation>
    with SingleTickerProviderStateMixin {
  final layoutManager = StartScreenLayoutManager();
  late final scrollController = SmoothScrollController(vsync: this);

  final contextController = FlyoutController();
  final contextAttachKey = GlobalKey();

  @override
  void dispose() {
    scrollController.dispose();
    layoutManager.dispose();
    contextController.dispose();
    super.dispose();
  }

  Future<void> scrollToGroup(String groupTitle) async {
    final data = Provider.of<CollectionDataProvider>(context, listen: false);

    bool lastPageReached = false;
    final padding = getScrollContainerPadding(context, listen: false);
    // Step 1: Check if the group already exists in the loaded items.
    while (!lastPageReached) {
      if (data.isLastPage) {
        lastPageReached = true;
      }

      final index =
          data.items.indexWhere((group) => group.groupTitle == groupTitle);

      // If found, calculate the scroll position.
      if (index != -1) {
        double scrollPosition = 0.0;

        // Step 5: Calculate the scroll position for the target group.
        for (int i = 0; i < index; i++) {
          final group = data.items[i];
          final dimensions =
              StartGroupImplementation.defaultDimensionCalculator(
            widget.constraints.maxHeight - padding.top - padding.bottom,
            defaultCellSize,
            4,
            group.items,
          );

          final (groupWidth, _) = StartGroupImplementation.finalSizeCalculator(
            dimensions,
            defaultCellSize,
            4,
          );

          scrollPosition += groupWidth + defaultGapSize + 16;
        }

        // Step 6: Scroll to the calculated position.
        scrollController.scrollTo(
          scrollPosition,
        );
        return;
      }

      // Step 2: If not found, load the next page.
      await data.fetchData();
    }

    // Step 3: If reached here, it means we didn't find the group and reached the last page.
    // Silent return as specified.
  }

  _fetchData() async {
    final data = Provider.of<CollectionDataProvider>(context, listen: false);
    await data.fetchData();

    Timer(
      Duration(milliseconds: gridAnimationDelay),
      () => layoutManager.playAnimations(),
    );
  }

  void openStartScreenContextMenu(
    Offset localPosition,
  ) async {
    if (!context.mounted) return;
    final targetContext = contextAttachKey.currentContext;

    if (targetContext == null) return;
    final box = targetContext.findRenderObject() as RenderBox;
    final position = box.localToGlobal(
      localPosition,
      ancestor: Navigator.of(context).context.findRenderObject(),
    );

    final data = Provider.of<CollectionDataProvider>(context, listen: false);

    contextController.showFlyout(
      position: position,
      builder: (context) {
        return MenuFlyout(
          items: [
            MenuFlyoutItem(
              leading: const Icon(Symbols.refresh),
              text: Text(S.of(context).refresh),
              onPressed: () async {
                layoutManager.resetAnimations();
                await data.reloadData();
                Timer(
                  Duration(milliseconds: gridAnimationDelay),
                  () => layoutManager.playAnimations(),
                );
              },
            ),
            if (data.collectionType == CollectionType.Mix)
              MenuFlyoutItem(
                leading: const Icon(Symbols.add),
                text: Text(S.of(context).newMix),
                onPressed: () async {
                  final x = await showMixStudioDialog(context);

                  if (x != null) data.reloadData();
                },
              ),
            if (data.collectionType == CollectionType.Playlist)
              MenuFlyoutItem(
                leading: const Icon(Symbols.add),
                text: Text(S.of(context).newPlaylist),
                onPressed: () async {
                  final x = await showCreateEditPlaylistDialog(context);

                  if (x != null) data.reloadData();
                },
              ),
          ],
        );
      },
    );
  }

  @override
  Widget build(BuildContext context) {
    final padding = getScrollContainerPadding(context);
    final c = widget.constraints;
    final trueConstraints = BoxConstraints(
      maxWidth: c.maxWidth - padding.left - padding.right,
      maxHeight: c.maxHeight - padding.top - padding.bottom,
    );

    final data = Provider.of<CollectionDataProvider>(context);
    final isUserGenerated = userGenerated(data.collectionType);

    return ChangeNotifierProvider<StartScreenLayoutManager>.value(
      value: layoutManager,
      child: FutureBuilder<List<Group<InternalCollection>>>(
        future: data.summary,
        builder: (context, snapshot) {
          if (snapshot.connectionState == ConnectionState.waiting) {
            return Container();
          } else if (snapshot.hasError) {
            return Center(child: Text('Error: ${snapshot.error}'));
          } else {
            return ContextMenuWrapper(
              contextAttachKey: contextAttachKey,
              contextController: contextController,
              onContextMenu: (offset) {
                openStartScreenContextMenu(offset);
              },
              onMiddleClick: (_) {},
              child: SmoothHorizontalScroll(
                controller: scrollController,
                builder: (context, smoothScrollController) {
                  return InfiniteList(
                    itemCount: data.items.length,
                    scrollDirection: Axis.horizontal,
                    scrollController: smoothScrollController,
                    loadingBuilder: (context) => const InfiniteListLoading(),
                    centerLoading: true,
                    centerEmpty: true,
                    isLoading: data.isLoading,
                    padding: padding,
                    emptyBuilder: (context) => Center(
                      child: data.initialized
                          ? NoItems(
                              title: S.of(context).noCollectionFound,
                              hasRecommendation: false,
                              reloadData: data.reloadData,
                              userGenerated: isUserGenerated,
                            )
                          : Container(),
                    ),
                    onFetchData: _fetchData,
                    hasReachedMax: data.isLastPage,
                    itemBuilder: (context, index) {
                      final item = data.items[index];
                      return StartGroup<InternalCollection>(
                        key: ValueKey(item.groupTitle),
                        groupIndex: index,
                        groupTitle: item.groupTitle,
                        items: item.items,
                        constraints: trueConstraints,
                        onTitleTap: () {
                          if (!isUserGenerated) {
                            showGroupListDialog(context, scrollToGroup);
                          }
                        },
                        itemBuilder: (context, item) =>
                            collectionItemBuilder(context, item),
                      );
                    },
                  );
                },
              ),
            );
          }
        },
      ),
    );
  }
}
