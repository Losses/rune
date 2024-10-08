import 'package:fluent_ui/fluent_ui.dart';
import 'package:infinite_scroll_pagination/infinite_scroll_pagination.dart';

import '../../utils/query_list.dart';
import '../../utils/queries_has_recommendation.dart';
import '../../widgets/library_task_button.dart';
import '../../widgets/no_items.dart';
import '../../widgets/smooth_horizontal_scroll.dart';
import '../start_screen/managed_start_screen_item.dart';

import './track_list_item.dart';

class InternalMediaFile {
  final int id;
  final String path;
  final String artist;
  final String album;
  final String title;
  final double duration;
  final String coverArtPath;

  InternalMediaFile({
    required this.id,
    required this.path,
    required this.artist,
    required this.album,
    required this.title,
    required this.duration,
    required this.coverArtPath,
  });
}

class TrackList extends StatelessWidget {
  final PagingController<int, InternalMediaFile> pagingController;
  final QueryList queries;
  final int mode;

  const TrackList({
    super.key,
    required this.pagingController,
    required this.queries,
    required this.mode,
  });

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.all(12),
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
                          title: "No tracks found",
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
                      child: TrackListItem(
                        index: index,
                        item: item,
                        queries: queries,
                        fallbackFileIds: fallbackFileIds,
                        coverArtPath: item.coverArtPath,
                        mode: mode,
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

class ActionButtons extends StatelessWidget {
  const ActionButtons({
    super.key,
    required this.reloadData,
    required this.hasRecommendation,
  });

  final VoidCallback reloadData;
  final bool hasRecommendation;

  @override
  Widget build(BuildContext context) {
    return Row(
      mainAxisAlignment: MainAxisAlignment.center,
      crossAxisAlignment: CrossAxisAlignment.center,
      children: [
        ScanLibraryButton(
          title: "Scan Library",
          onFinished: reloadData,
        ),
        if (hasRecommendation) ...[
          const SizedBox(width: 12),
          AnalyseLibraryButton(
            title: "Analyse Tracks",
            onFinished: reloadData,
          ),
        ]
      ],
    );
  }
}
