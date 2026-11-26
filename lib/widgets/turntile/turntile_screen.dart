import '../../utils/api/request_failure.dart';
import 'dart:async';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:very_good_infinite_list/very_good_infinite_list.dart';

import '../../utils/dialogs/show_group_list_dialog.dart';
import '../../config/animation.dart';
import '../../screens/collection/utils/is_user_generated.dart';
import '../../screens/collection/utils/collection_item_builder.dart';
import '../../screens/collection/utils/group_title.dart';
import '../../widgets/no_items.dart';
import '../../screens/collection/utils/collection_data_provider.dart';
import '../../utils/l10n.dart';

import '../infinite_list_loading.dart';
import '../start_screen/utils/group.dart';
import '../start_screen/utils/internal_collection.dart';
import '../start_screen/providers/start_screen_layout_manager.dart';
import '../navigation_bar/page_content_frame.dart';

import 'turntile_group.dart';

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

  TurntileGroup<InternalCollection> _buildGroup(
    BuildContext context,
    CollectionDataProvider data,
    int index,
  ) {
    final item = data.items[index];
    final isUserGenerated = userGenerated(data.collectionType);
    return TurntileGroup<InternalCollection>(
      key: Key(item.groupTitle),
      groupIndex: index,
      groupTitle: collectionGroupTitle(context, item),
      gridLayoutVariation: TurntileGroupGridLayoutVariation.tile,
      items: item.items,
      onTitleTap: () {
        if (!isUserGenerated) {
          showGroupListDialog(context, scrollToGroup);
        }
      },
      itemBuilder: (context, item) => collectionItemBuilder(context, item),
    );
  }

  @override
  void scrollToGroupIndex(CollectionDataProvider data, int index) {
    final padding = getScrollContainerPadding(context, listen: false);
    final listWidth =
        widget.constraints.maxWidth - padding.left - padding.right;
    double scrollPosition = 0.0;

    for (int i = 0; i < index; i++) {
      scrollPosition +=
          _buildGroup(context, data, i).heightIn(context, listWidth);
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
    final showEmptyDetail = emptyPageShowsDetail(context, data.collectionType);
    final pageError = data.pageError;

    return ChangeNotifierProvider<StartScreenLayoutManager>.value(
      value: layoutManager,
      child: FutureBuilder<List<Group<InternalCollection>>>(
        future: data.summary,
        builder: (context, snapshot) {
          // A failed summary keeps its error while the next one loads, so the
          // spinner also waits for the items: those that stayed are shown.
          if (snapshot.connectionState == ConnectionState.waiting &&
              !snapshot.hasData &&
              data.items.isEmpty) {
            return const Center(child: ProgressRing());
          } else if (snapshot.hasError && data.items.isEmpty) {
            // Only with nothing on screen: a reload whose summary fails keeps
            // its items.
            return Center(child: Text(requestErrorText(S.of(context), snapshot.error!)));
          } else {
            return InfiniteList(
              itemCount: data.items.length,
              // Nothing is drawn at the end while the next page loads.
              loadingBuilder: (context) => data.items.isEmpty
                  ? const InfiniteListLoading()
                  : const SizedBox.shrink(),
              cacheExtent: lazyLoadCacheExtent(context, Axis.vertical),
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
                        showDetail: showEmptyDetail,
                      )
                    : Container(),
              ),
              onFetchData: _fetchData,
              hasReachedMax: data.isLastPage,
              itemBuilder: (context, index) {
                return _buildGroup(context, data, index);
              },
            );
          }
        },
      ),
    );
  }
}
