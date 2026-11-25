import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/l10n.dart';
import '../../bindings/bindings.dart';
import '../../utils/query_list.dart';
import '../../utils/queries_has_recommendation.dart';
import '../../widgets/no_items.dart';
import '../../widgets/ax_pressure.dart';
import '../../widgets/smooth_horizontal_scroll.dart';
import '../../widgets/track_list/utils/internal_media_file.dart';

import '../ax_reveal/ax_reveal.dart';
import '../navigation_bar/page_content_frame.dart';
import '../start_screen/managed_start_screen_item.dart';
import '../start_screen/stacked_layout.dart';

import 'large_screen_track_list_item.dart';

/// The height of a tile of the large track list, and how many times narrower
/// than wide it is.
const double largeTrackListCellSize = 64;
const double largeTrackListRatio = 1 / 4;

/// The width of one tile of the large track list.
const double largeTrackListTileWidth =
    largeTrackListCellSize / largeTrackListRatio;

/// The padding the list adds to the right of its last column, over what a
/// scrolling list takes from its page.
const double largeTrackListRightPlus = 12;

class LargeScreenTrackList extends StatelessWidget {
  final int totalCount;
  final InternalMediaFile? Function(int) getItem;
  final QueryList queries;
  final PlaybackMode? mode;
  /// Lists the tracks again, after a scan started from the empty list.
  final VoidCallback reloadData;

  /// Set when the list is the child of a [StartGroupStackedLayout], which
  /// already pads its child. The list then adds no padding above and none to
  /// the left of its first column.
  final bool underTitle;

  const LargeScreenTrackList({
    super.key,
    required this.totalCount,
    required this.getItem,
    required this.queries,
    required this.mode,
    required this.reloadData,
    this.underTitle = false,
  });

  @override
  Widget build(BuildContext context) {
    final isAlbumQuery = QueryList.computeIsAlbumQuery(queries);

    return Padding(
      padding: underTitle
          ? const EdgeInsets.only(bottom: 12)
          : const EdgeInsets.symmetric(vertical: 12),
      child: LayoutBuilder(
        builder: (context, constraints) {
          const double gapSize = 8;
          const double cellSize = largeTrackListCellSize;

          final int rows = (constraints.maxHeight / (cellSize + gapSize))
              .floor();
          final double finalHeight = rows * (cellSize + gapSize) - gapSize;

          const ratio = largeTrackListRatio;

          final hasRecommendation = queriesHasRecommendation(queries);

          if (totalCount == 0) {
            // Without the grid, no scroll view would use a scrollbar's
            // controller.
            return SizedBox(
              height: finalHeight,
              child: Center(
                child: NoItems(
                  title: S.of(context).noTracksFound,
                  hasRecommendation: hasRecommendation,
                  reloadData: reloadData,
                ),
              ),
            );
          }

          return SmoothHorizontalScroll(
            builder: (context, scrollController) => Scrollbar(
              controller: scrollController,
              thumbVisibility: true,
              child: SizedBox(
                height: finalHeight,
                child: GridView.builder(
                  padding: getScrollContainerPadding(
                    context,
                    leftPlus: underTitle ? 0 : 12,
                    rightPlus: largeTrackListRightPlus,
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

                    // A track that has not loaded leaves its tile blank.
                    if (item == null) return const SizedBox.shrink();

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
                            solidBackground: underTitle,
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
