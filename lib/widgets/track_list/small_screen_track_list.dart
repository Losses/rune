import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/query_list.dart';
import '../../utils/queries_has_recommendation.dart';
import '../../widgets/no_items.dart';
import '../../widgets/ax_pressure.dart';
import '../../widgets/turntile/managed_turntile_screen_item.dart';
import '../../widgets/track_list/small_screen_track_list_item.dart';
import '../../widgets/track_list/utils/internal_media_file.dart';
import '../../utils/l10n.dart';

import '../navigation_bar/page_content_frame.dart';

class SmallScreenTrackList extends StatefulWidget {
  final int totalCount;
  final InternalMediaFile? Function(int) getItem;
  final QueryList queries;
  final int mode;
  final bool topPadding;

  const SmallScreenTrackList({
    super.key,
    required this.totalCount,
    required this.getItem,
    required this.queries,
    required this.mode,
    required this.topPadding,
  });

  @override
  State<SmallScreenTrackList> createState() => _SmallScreenTrackListState();
}

class _SmallScreenTrackListState extends State<SmallScreenTrackList> {
  final ScrollController _scrollController = ScrollController();

  @override
  void dispose() {
    _scrollController.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final hasRecommendation = queriesHasRecommendation(widget.queries);

    if (widget.totalCount == 0) {
      return Center(
        child: NoItems(
          title: S.of(context).noTracksFound,
          hasRecommendation: hasRecommendation,
          reloadData: () {},
        ),
      );
    }

    return Scrollbar(
      controller: _scrollController,
      thumbVisibility: true,
      child: ListView.builder(
        controller: _scrollController,
        padding: getScrollContainerPadding(
          context,
          top: widget.topPadding,
          leftPlus: 16,
          rightPlus: 16,
        ),
        itemCount: widget.totalCount,
        itemBuilder: (context, index) {
          final item = widget.getItem(index);

          if (item == null) {
            return const SizedBox(
              height: 64,
              child: Center(child: ProgressRing()),
            );
          }

          final isLastItem = index == widget.totalCount - 1;

          return Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              ManagedTurntileScreenItem(
                groupId: 0,
                row: index,
                column: 1,
                child: AxPressure(
                  child: SmallScreenTrackListItem(
                    index: index,
                    item: item,
                    queries: widget.queries,
                    fallbackFileIds: const [],
                    coverArtPath: item.coverArtPath,
                    mode: widget.mode,
                    position: index,
                    reloadData: () {},
                  ),
                ),
              ),
              // Add extra bottom spacing for the last item
              if (isLastItem)
                SizedBox(height: MediaQuery.of(context).size.width / 3 + 20),
            ],
          );
        },
      ),
    );
  }
}
