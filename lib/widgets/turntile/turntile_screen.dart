import '../../utils/api/request_failure.dart';
import 'dart:async';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:very_good_infinite_list/very_good_infinite_list.dart';

import '../../utils/dialogs/show_group_list_dialog.dart';
import '../../config/animation.dart';
import '../../screens/collection/utils/is_user_generated.dart';
import '../../screens/collection/utils/collection_item_builder.dart';
import '../../widgets/no_items.dart';
import '../../screens/collection/utils/collection_data_provider.dart';
import '../../utils/l10n.dart';

import '../infinite_list_loading.dart';
import '../start_screen/utils/group.dart';
import '../start_screen/utils/internal_collection.dart';
import '../start_screen/providers/start_screen_layout_manager.dart';
import '../navigation_bar/page_content_frame.dart';

import 'turntile_group.dart';
import 'turntile_group_items_tile.dart';

class TurntileScreen extends StatelessWidget {
  const TurntileScreen({super.key});

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        return TurntileScreenImplementation(
          constraints: constraints,
        );
      },
    );
  }
}

class TurntileScreenImplementation extends StatefulWidget {
  final BoxConstraints constraints;

  const TurntileScreenImplementation({
    super.key,
    required this.constraints,
  });

  @override
  TurntileScreenImplementationState createState() =>
      TurntileScreenImplementationState();
}

class TurntileScreenImplementationState
    extends State<TurntileScreenImplementation>
    with CollectionGroupNavigation<TurntileScreenImplementation> {
  final layoutManager = StartScreenLayoutManager();
  final scrollController = ScrollController();

  @override
  void dispose() {
    super.dispose();
    layoutManager.dispose();
    scrollController.dispose();
  }

  @override
  void scrollToGroupIndex(CollectionDataProvider data, int index) {
    final padding = getScrollContainerPadding(context, listen: false);
    double scrollPosition = 0.0;

    final containerWidth = widget.constraints.maxWidth -
        padding.left -
        padding.right -
        16 -
        16;

    for (int i = 0; i < index; i++) {
      final group = data.items[i];
      final dimensions =
          TurntileGroupItemsTile.defaultTurntileDimensionCalculator(
        containerWidth,
        88,
        4,
        group.items,
      );

      final finalCellSize =
          (containerWidth - (dimensions.columns * 4)) / dimensions.columns;

      final double finalHeight =
          finalCellSize * dimensions.rows + 4 * (dimensions.rows - 1);

      scrollPosition += finalHeight + 4 + 37;
    }

    final deltaPosition = (scrollController.offset - scrollPosition).abs();
    if (deltaPosition == 0) return;
    scrollController.animateTo(
      scrollPosition,
      duration: Duration(milliseconds: (deltaPosition / 30).ceil()),
      curve: Curves.easeOutQuint,
    );
  }

  Future<void> _fetchData({bool retry = false}) async {
    if (!mounted) return;
    final data = Provider.of<CollectionDataProvider>(context, listen: false);
    if (data.pageError != null && !retry) return;
    await data.fetchData();
    if (!mounted || data.pageError != null) return;

    Timer(
      Duration(milliseconds: gridAnimationDelay),
      () => layoutManager.playAnimations(),
    );
  }

  @override
  Widget build(BuildContext context) {
    final data = Provider.of<CollectionDataProvider>(context);
    final isUserGenerated = userGenerated(data.collectionType);
    final pageError = data.pageError;

    return ChangeNotifierProvider<StartScreenLayoutManager>.value(
      value: layoutManager,
      child: FutureBuilder<List<Group<InternalCollection>>>(
        future: data.summary,
        builder: (context, snapshot) {
          if (snapshot.connectionState == ConnectionState.waiting &&
              !snapshot.hasData) {
            return const Center(child: ProgressRing());
          } else if (snapshot.hasError) {
            return Center(child: Text(requestErrorText(S.of(context), snapshot.error!)));
          } else {
            return InfiniteList(
              itemCount: data.items.length,
              loadingBuilder: (context) => const InfiniteListLoading(),
              centerLoading: true,
              centerEmpty: true,
              isLoading: data.isLoading,
              hasError: pageError != null,
              centerError: data.items.isEmpty,
              errorBuilder: (_) => InfiniteListRetry(
                error: pageError!,
                onRetry: () => retryGroupOrPage(() => _fetchData(retry: true)),
              ),
              padding: getScrollContainerPadding(context),
              scrollController: scrollController,
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
                return TurntileGroup<InternalCollection>(
                  key: Key(item.groupTitle),
                  groupIndex: index,
                  groupTitle: item.groupTitle,
                  gridLayoutVariation: TurntileGroupGridLayoutVariation.tile,
                  items: item.items,
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
          }
        },
      ),
    );
  }
}
