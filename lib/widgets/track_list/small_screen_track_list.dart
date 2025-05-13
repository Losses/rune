import 'package:fluent_ui/fluent_ui.dart';
import 'package:infinite_scroll_pagination/infinite_scroll_pagination.dart';

import '../../utils/query_list.dart';
import '../../utils/queries_has_recommendation.dart';
import '../../widgets/no_items.dart';
import '../../widgets/ax_pressure.dart';
import '../../widgets/turntile/managed_turntile_screen_item.dart';
import '../../widgets/track_list/small_screen_track_list_item.dart';
import '../../widgets/track_list/utils/internal_media_file.dart';
import '../../utils/l10n.dart';

import '../navigation_bar/page_content_frame.dart';

class SmallScreenTrackList extends StatelessWidget {
  final PagingController<int, InternalMediaFile> pagingController;
  final QueryList queries;
  final int mode;
  final bool topPadding;

  const SmallScreenTrackList({
    super.key,
    required this.pagingController,
    required this.queries,
    required this.mode,
    required this.topPadding,
  });

  @override
  Widget build(BuildContext context) {
    final hasRecommendation = queriesHasRecommendation(queries);
    final fallbackFileIds =
        pagingController.itemList?.map((x) => x.id).toList() ?? [];

    return PagedListView<int, InternalMediaFile>(
      pagingController: pagingController,
      padding: getScrollContainerPadding(
        context,
        top: topPadding,
        leftPlus: 16,
        rightPlus: 16,
      ),
      builderDelegate: PagedChildBuilderDelegate<InternalMediaFile>(
        noItemsFoundIndicatorBuilder: (context) {
          return SizedBox.expand(
            child: Center(
              child: NoItems(
                title: S.of(context).noTracksFound,
                hasRecommendation: hasRecommendation,
                reloadData: pagingController.refresh,
              ),
            ),
          );
        },
        itemBuilder: (context, item, index) {
          return ManagedTurntileScreenItem(
            groupId: 0,
            row: index,
            column: 1,
            child: AxPressure(
              child: SmallScreenTrackListItem(
                index: index,
                item: item,
                queries: queries,
                fallbackFileIds: fallbackFileIds,
                coverArtPath: item.coverArtPath,
                mode: mode,
                position: index,
                reloadData: pagingController.refresh,
              ),
            ),
          );
        },
      ),
    );
  }
}
