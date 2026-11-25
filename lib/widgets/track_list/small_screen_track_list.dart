import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/query_list.dart';
import '../../utils/queries_has_recommendation.dart';
import '../../widgets/no_items.dart';
import '../../widgets/ax_pressure.dart';
import '../../widgets/turntile/managed_turntile_screen_item.dart';
import '../../widgets/track_list/small_screen_track_list_item.dart';
import '../../widgets/track_list/utils/internal_media_file.dart';
import '../../utils/l10n.dart';
import '../../bindings/bindings.dart';

import '../navigation_bar/page_content_frame.dart';

class SmallScreenTrackList extends StatefulWidget {
  final int totalCount;
  final InternalMediaFile? Function(int) getItem;
  final QueryList queries;
  final PlaybackMode? mode;
  /// Lists the tracks again, after a scan started from the empty list.
  final VoidCallback reloadData;

  /// Drawn as the list's first item, scrolling away with the tracks. It gets
  /// the width inside the list's own padding and pads itself, as a group of
  /// the turntile screen does, so the tracks are padded by 16 on both sides
  /// as the header is. A list with no tracks then shows only the header.
  final Widget? header;

  const SmallScreenTrackList({
    super.key,
    required this.totalCount,
    required this.getItem,
    required this.queries,
    required this.mode,
    required this.reloadData,
    this.header,
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

    if (widget.totalCount == 0 && widget.header == null) {
      return Center(
        child: NoItems(
          title: S.of(context).noTracksFound,
          hasRecommendation: hasRecommendation,
          reloadData: widget.reloadData,
        ),
      );
    }

    final header = widget.header;

    return Scrollbar(
      controller: _scrollController,
      thumbVisibility: true,
      child: ListView.builder(
        controller: _scrollController,
        padding: header == null
            ? getScrollContainerPadding(context, leftPlus: 16, rightPlus: 16)
            : getScrollContainerPadding(context),
        itemCount: widget.totalCount + (header == null ? 0 : 1),
        itemBuilder: (context, position) {
          if (header != null && position == 0) return header;
          final index = header == null ? position : position - 1;
          final item = widget.getItem(index);

          // A track that has not loaded leaves its row blank.
          if (item == null) return const SizedBox(height: 64);

          final tile = ManagedTurntileScreenItem(
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
          );
          return header == null
              ? tile
              : Padding(
                  padding: const EdgeInsets.symmetric(horizontal: 16),
                  child: tile,
                );
        },
      ),
    );
  }
}
