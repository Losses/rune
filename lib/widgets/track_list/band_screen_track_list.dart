import 'package:fluent_ui/fluent_ui.dart';
import 'package:infinite_scroll_pagination/infinite_scroll_pagination.dart';

import '../../utils/query_list.dart';
import '../../utils/format_time.dart';
import '../../utils/queries_has_recommendation.dart';
import '../../utils/context_menu/track_item_context_menu.dart';
import '../../utils/api/operate_playback_with_mix_query.dart';
import '../../widgets/no_items.dart';
import '../../widgets/ax_pressure.dart';
import '../../widgets/tile/tile.dart';
import '../../widgets/turntile/managed_turntile_screen_item.dart';
import '../../widgets/track_list/utils/internal_media_file.dart';

import '../context_menu_wrapper.dart';
import '../tile/cover_art.dart';

class BandScreenTrackList extends StatelessWidget {
  final PagingController<int, InternalMediaFile> pagingController;
  final QueryList queries;
  final int mode;

  BandScreenTrackList({
    super.key,
    required this.pagingController,
    required this.queries,
    required this.mode,
  });

  final contextController = FlyoutController();
  final contextAttachKey = GlobalKey();

  @override
  Widget build(BuildContext context) {
    final hasRecommendation = queriesHasRecommendation(queries);

    return PagedListView<int, InternalMediaFile>(
      pagingController: pagingController,
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
          return ManagedTurntileScreenItem(
            groupId: 0,
            row: index,
            column: 1,
            child: AspectRatio(
              aspectRatio: 1,
              child: Padding(
                padding: const EdgeInsets.symmetric(
                  horizontal: 2,
                  vertical: 1,
                ),
                child: AxPressure(
                  child: ContextMenuWrapper(
                    contextAttachKey: contextAttachKey,
                    contextController: contextController,
                    onContextMenu: (position) {
                      openTrackItemContextMenu(
                        position,
                        context,
                        contextAttachKey,
                        contextController,
                        item.id,
                      );
                    },
                    child: Tile(
                      onPressed: () {
                        safeOperatePlaybackWithMixQuery(
                          context: context,
                          queries: queries,
                          playbackMode: mode,
                          hintPosition: index,
                          initialPlaybackId: item.id,
                          replacePlaylist: true,
                          instantlyPlay: true,
                          fallbackFileIds: pagingController.itemList
                                  ?.map((x) => x.id)
                                  .toList() ??
                              [],
                        );
                      },
                      child: CoverArt(
                        path: item.coverArtPath,
                        size: 40,
                        hint: (
                          item.album,
                          item.artist,
                          'Total Time ${formatTime(item.duration)}'
                        ),
                      ),
                    ),
                  ),
                ),
              ),
            ),
          );
        },
      ),
    );
  }
}
