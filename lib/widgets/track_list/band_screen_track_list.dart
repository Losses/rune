import 'package:fluent_ui/fluent_ui.dart';
import 'package:infinite_scroll_pagination/infinite_scroll_pagination.dart';

import '../../utils/query_list.dart';
import '../../utils/format_time.dart';
import '../../utils/playing_item.dart';
import '../../utils/queries_has_recommendation.dart';
import '../../utils/execute_middle_click_action.dart';
import '../../utils/get_playlist_id_from_query_list.dart';
import '../../utils/api/operate_playback_with_mix_query.dart';
import '../../utils/router/router_aware_flyout_controller.dart';
import '../../utils/context_menu/track_item_context_menu.dart';
import '../../widgets/no_items.dart';
import '../../widgets/ax_pressure.dart';
import '../../widgets/smooth_horizontal_scroll.dart';
import '../../widgets/tile/tile.dart';
import '../../widgets/turntile/managed_turntile_screen_item.dart';
import '../../widgets/track_list/utils/internal_media_file.dart';
import '../../bindings/bindings.dart';
import '../../providers/responsive_providers.dart';
import '../../utils/l10n.dart';

import '../context_menu_wrapper.dart';
import '../navigation_bar/page_content_frame.dart';
import '../tile/cover_art.dart';

class BandScreenTrackList extends StatelessWidget {
  final PagingController<int, InternalMediaFile> pagingController;
  final QueryList queries;
  final int mode;
  final bool topPadding;

  const BandScreenTrackList({
    super.key,
    required this.pagingController,
    required this.queries,
    required this.mode,
    required this.topPadding,
  });

  @override
  Widget build(BuildContext context) {
    final hasRecommendation = queriesHasRecommendation(queries);

    return DeviceTypeBuilder(
      deviceType: const [
        DeviceType.band,
        DeviceType.belt,
        DeviceType.dock,
        DeviceType.tv
      ],
      builder: (context, deviceType) {
        if (deviceType == DeviceType.band || deviceType == DeviceType.belt) {
          return SmoothHorizontalScroll(
            builder: (context, controller) {
              return buildList(context, hasRecommendation, controller);
            },
          );
        } else {
          return buildList(context, hasRecommendation, null);
        }
      },
    );
  }

  PagedListView<int, InternalMediaFile> buildList(
    BuildContext context,
    bool hasRecommendation,
    ScrollController? scrollController,
  ) {
    return PagedListView<int, InternalMediaFile>(
      scrollDirection:
          scrollController == null ? Axis.vertical : Axis.horizontal,
      pagingController: pagingController,
      scrollController: scrollController,
      padding: getScrollContainerPadding(context, top: topPadding),
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
          return BandViewTrackItem(
            index: index,
            item: item,
            queries: queries,
            mode: mode,
            pagingController: pagingController,
            position: index,
          );
        },
      ),
    );
  }
}

class BandViewTrackItem extends StatefulWidget {
  const BandViewTrackItem({
    super.key,
    required this.index,
    required this.item,
    required this.queries,
    required this.mode,
    required this.pagingController,
    required this.position,
  });

  final int index;
  final InternalMediaFile item;
  final QueryList queries;
  final int mode;
  final PagingController<int, InternalMediaFile> pagingController;
  final int position;

  @override
  State<BandViewTrackItem> createState() => _BandViewTrackItemState();
}

class _BandViewTrackItemState extends State<BandViewTrackItem> {
  final _contextController = RouterAwareFlyoutController();
  final _contextAttachKey = GlobalKey();

  @override
  void dispose() {
    super.dispose();
    _contextController.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return ManagedTurntileScreenItem(
      groupId: 0,
      row: widget.index,
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
              contextAttachKey: _contextAttachKey,
              contextController: _contextController,
              onMiddleClick: (_) {
                executeMiddleClickAction(
                  context,
                  CollectionType.track,
                  widget.item.id,
                );
              },
              onContextMenu: (position) {
                final playlistId = getPlaylistIdFromQueryList(widget.queries);
                openTrackItemContextMenu(
                  position,
                  context,
                  _contextAttachKey,
                  _contextController,
                  widget.position,
                  widget.item.id,
                  playlistId,
                  widget.pagingController.refresh,
                );
              },
              child: Tile(
                onPressed: () {
                  safeOperatePlaybackWithMixQuery(
                    context: context,
                    queries: widget.queries,
                    playbackMode: widget.mode,
                    hintPosition: widget.index,
                    initialPlaybackId: widget.item.id,
                    operateMode: PlaylistOperateMode.replace,
                    instantlyPlay: true,
                    fallbackPlayingItems: widget.pagingController.itemList
                            ?.map((x) => x.id)
                            .map(PlayingItem.inLibrary)
                            .toList() ??
                        [],
                  );
                },
                child: CoverArt(
                  path: widget.item.coverArtPath,
                  hint: (
                    widget.item.album,
                    widget.item.artist,
                    'Total Time ${formatTime(widget.item.duration)}'
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}
