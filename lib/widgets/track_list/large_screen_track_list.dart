import 'package:fluent_ui/fluent_ui.dart';

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
  final int totalCount;
  final InternalMediaFile? Function(int) getItem;
  final QueryList queries;
  final int mode;
  final bool topPadding;

  const LargeScreenTrackList({
    super.key,
    required this.totalCount,
    required this.getItem,
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

          final int rows = (constraints.maxHeight / (cellSize + gapSize))
              .floor();
          final double finalHeight = rows * (cellSize + gapSize) - gapSize;

          const ratio = 1 / 4;

          final hasRecommendation = queriesHasRecommendation(queries);

          return SmoothHorizontalScroll(
            builder: (context, scrollController) => Scrollbar(
              controller: scrollController,
              thumbVisibility: true,
              child: SizedBox(
                height: finalHeight,
                child: totalCount == 0
                    ? Center(
                        child: NoItems(
                          title: S.of(context).noTracksFound,
                          hasRecommendation: hasRecommendation,
                          reloadData: () {},
                        ),
                      )
                    : GridView.builder(
                        padding: getScrollContainerPadding(
                          context,
                          top: topPadding,
                          leftPlus: 12,
                          rightPlus: 12,
                        ),
                        scrollDirection: Axis.horizontal,
                        controller: scrollController,
                        gridDelegate: SliverGridDelegateWithFixedCrossAxisCount(
                          crossAxisCount: rows,
                          mainAxisSpacing: gapSize,
                          crossAxisSpacing: gapSize,
                          childAspectRatio: ratio,
                        ),
                        itemCount: totalCount,
                        itemBuilder: (context, index) {
                          final item = getItem(index);

                          if (item == null) {
                            return const Center(child: ProgressRing());
                          }

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
                                  fallbackFileIds: const [],
                                  coverArtPath: item.coverArtPath,
                                  mode: mode,
                                  isAlbumQuery: isAlbumQuery,
                                  position: index,
                                  reloadData: () {},
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
