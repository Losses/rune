import 'package:fluent_ui/fluent_ui.dart';
import 'package:infinite_scroll_pagination/infinite_scroll_pagination.dart';

import '../../widgets/start_screen/providers/managed_start_screen_item.dart';
import '../../widgets/smooth_horizontal_scroll.dart';
import '../../messages/media_file.pb.dart';

import './track_list_item.dart';

class TrackList extends StatelessWidget {
  final PagingController<int, MediaFile> pagingController;

  const TrackList({super.key, required this.pagingController});

  @override
  Widget build(BuildContext context) {
    return Padding(
        padding: const EdgeInsets.all(12),
        child: LayoutBuilder(builder: (context, constraints) {
          const double gapSize = 8;
          const double cellSize = 64;

          final int rows =
              (constraints.maxHeight / (cellSize + gapSize)).floor();
          final double finalHeight = rows * (cellSize + gapSize) - gapSize;

          const ratio = 1 / 4;

          return SmoothHorizontalScroll(
              builder: (context, scrollController) => SizedBox(
                    height: finalHeight,
                    child: PagedGridView<int, MediaFile>(
                      pagingController: pagingController,
                      scrollDirection: Axis.horizontal,
                      scrollController: scrollController,
                      gridDelegate: SliverGridDelegateWithFixedCrossAxisCount(
                        crossAxisCount: rows,
                        mainAxisSpacing: gapSize,
                        crossAxisSpacing: gapSize,
                        childAspectRatio: ratio,
                      ),
                      builderDelegate: PagedChildBuilderDelegate<MediaFile>(
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
                              ));
                        },
                      ),
                    ),
                  ));
        }));
  }
}
