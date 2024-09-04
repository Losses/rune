import 'package:fluent_ui/fluent_ui.dart';
import 'package:infinite_scroll_pagination/infinite_scroll_pagination.dart';

import '../../../../widgets/smooth_horizontal_scroll.dart';
import '../../../../messages/media_file.pb.dart';

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

        final int rows = (constraints.maxHeight / (cellSize + gapSize)).floor();
        final double finalHeight = rows * (cellSize + gapSize) - gapSize;

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
                childAspectRatio: 1 / 4,
              ),
              builderDelegate: PagedChildBuilderDelegate<MediaFile>(
                itemBuilder: (context, item, index) => TrackListItem(
                  index: index,
                  item: item,
                ),
              ),
            ),
          ),
        );
      }),
    );
  }
}
