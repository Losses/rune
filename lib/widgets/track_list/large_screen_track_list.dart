import 'package:fluent_ui/fluent_ui.dart';
import 'package:infinite_scroll_pagination/infinite_scroll_pagination.dart';

import '../../utils/l10n.dart';
import '../../utils/query_list.dart';
import '../../utils/queries_has_recommendation.dart';
import '../../widgets/no_items.dart';
import '../../widgets/ax_pressure.dart';
import '../../widgets/smooth_horizontal_scroll.dart';
import '../../widgets/track_list/utils/internal_media_file.dart';

import '../ax_reveal/ax_reveal.dart';
import '../navigation_bar/page_content_frame.dart';
import '../start_screen/managed_start_screen_item.dart';

import 'large_screen_track_list_item.dart';

class LargeScreenTrackList extends StatelessWidget {
  final PagingController<int, InternalMediaFile> pagingController;
  final QueryList queries;
  final int mode;
  final bool topPadding;

  const LargeScreenTrackList({
    super.key,
    required this.pagingController,
    required this.queries,
    required this.mode,
    required this.topPadding,
  });

  @override
  Widget build(BuildContext context) {
    final isAlbumQuery = QueryList.computeIsAlbumQuery(queries);

    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 12),
      child: LayoutBuilder(
        builder: (context, constraints) {
          const double gapSize = 8;
          const double cellSize = 64;

          final int rows =
              (constraints.maxHeight / (cellSize + gapSize)).floor();
          final double finalHeight = rows * (cellSize + gapSize) - gapSize;

          const ratio = 1 / 4;

          final hasRecommendation = queriesHasRecommendation(queries);
          final fallbackFileIds =
              pagingController.itemList?.map((x) => x.id).toList() ?? [];

          return SmoothHorizontalScroll(
            builder: (context, scrollController) => SizedBox(
              height: finalHeight,
              child: PagedGridView<int, InternalMediaFile>(
                pagingController: pagingController,
                padding: getScrollContainerPadding(
                  context,
                  top: topPadding,
                  leftPlus: 12,
                  rightPlus: 12,
                ),
                scrollDirection: Axis.horizontal,
                scrollController: scrollController,
                gridDelegate: SliverGridDelegateWithFixedCrossAxisCount(
                  crossAxisCount: rows,
                  mainAxisSpacing: gapSize,
                  crossAxisSpacing: gapSize,
                  childAspectRatio: ratio,
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
                    final int row = index % rows;
                    final int column = index ~/ rows;

                    return ManagedStartScreenItem(
                      groupId: 0,
                      row: row,
                      column: column,
                      width: cellSize / ratio,
                      height: cellSize,
                      child: AxPressure(
                        child: AxReveal0(
                          child: LargeScreenTrackListItem(
                            index: index,
                            item: item,
                            queries: queries,
                            fallbackFileIds: fallbackFileIds,
                            coverArtPath: item.coverArtPath,
                            mode: mode,
                            isAlbumQuery: isAlbumQuery,
                          ),
                        ),
                      ),
                    );
                  },
                ),
              ),
            ),
          );
        },
      ),
    );
  }
}
