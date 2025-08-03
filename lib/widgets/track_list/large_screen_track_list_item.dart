import 'dart:math';
import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/query_list.dart';
import '../../utils/format_time.dart';
import '../../utils/playing_item.dart';
import '../../utils/execute_middle_click_action.dart';
import '../../utils/get_playlist_id_from_query_list.dart';
import '../../utils/api/operate_playback_with_mix_query.dart';
import '../../utils/context_menu/track_item_context_menu.dart';
import '../../utils/router/router_aware_flyout_controller.dart';
import '../../widgets/context_menu_wrapper.dart';
import '../../widgets/track_list/utils/internal_media_file.dart';
import '../../bindings/bindings.dart';

import '../tile/cover_art.dart';
import 'widgets/track_number_cover_art.dart';

class LargeScreenTrackListItem extends StatefulWidget {
  final InternalMediaFile item;
  final int index;
  final QueryList queries;
  final int mode;
  final String? coverArtPath;
  final List<int> fallbackFileIds;
  final bool isAlbumQuery;
  final int position;
  final void Function()? reloadData;

  const LargeScreenTrackListItem({
    super.key,
    required this.index,
    required this.item,
    required this.queries,
    required this.mode,
    required this.fallbackFileIds,
    required this.coverArtPath,
    required this.isAlbumQuery,
    required this.position,
    required this.reloadData,
  });

  @override
  State<LargeScreenTrackListItem> createState() =>
      _LargeScreenTrackListItemState();
}

class _LargeScreenTrackListItemState extends State<LargeScreenTrackListItem> {
  final _contextController = RouterAwareFlyoutController();
  final _contextAttachKey = GlobalKey();

  @override
  dispose() {
    super.dispose();
    _contextController.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final Typography typography = FluentTheme.of(context).typography;
    final int? trackNumber =
        widget.item.trackNumber == 0 ? null : widget.item.trackNumber;
    final int? diskNumber =
        trackNumber == null ? trackNumber : trackNumber ~/ 1000;

    return ContextMenuWrapper(
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
          widget.reloadData,
        );
      },
      child: Button(
        style: const ButtonStyle(
          padding: WidgetStatePropertyAll(EdgeInsets.all(0)),
        ),
        onPressed: () async {
          if (context.mounted) {
            await safeOperatePlaybackWithMixQuery(
              context: context,
              queries: widget.queries,
              playbackMode: widget.mode,
              hintPosition: widget.index,
              initialPlaybackId: widget.item.id,
              operateMode: PlaylistOperateMode.replace,
              instantlyPlay: true,
              fallbackPlayingItems:
                  widget.fallbackFileIds.map(PlayingItem.inLibrary).toList(),
            );
          }
        },
        child: ClipRRect(
          borderRadius: BorderRadius.circular(3),
          child: LayoutBuilder(
            builder: (context, constraints) {
              final size = min(constraints.maxWidth, constraints.maxHeight);
              return Row(
                children: [
                  widget.isAlbumQuery && trackNumber != null
                      ? TrackNumberCoverArt(
                          diskNumber: diskNumber == 0 ? null : diskNumber,
                          trackNumber: trackNumber % 1000,
                        )
                      : CoverArt(
                          path: widget.coverArtPath,
                          size: size,
                          hint: (
                            widget.item.album,
                            widget.item.artist,
                            'Total Time ${formatTime(widget.item.duration)}'
                          ),
                        ),
                  Expanded(
                    child: Padding(
                      padding: const EdgeInsets.all(8),
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        mainAxisAlignment: MainAxisAlignment.center,
                        children: [
                          Text(
                            widget.item.title,
                            overflow: TextOverflow.ellipsis,
                          ),
                          const SizedBox(height: 4),
                          Text(
                            widget.item.artist,
                            style: typography.caption?.apply(
                              color: typography.caption?.color?.withAlpha(117),
                            ),
                            overflow: TextOverflow.ellipsis,
                          ),
                        ],
                      ),
                    ),
                  ),
                ],
              );
            },
          ),
        ),
      ),
    );
  }
}
