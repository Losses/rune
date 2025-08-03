import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/execute_middle_click_action.dart';
import '../../../utils/playing_item.dart';
import '../../../utils/query_list.dart';
import '../../../utils/format_time.dart';
import '../../../utils/api/operate_playback_with_mix_query.dart';
import '../../../utils/context_menu/track_item_context_menu.dart';
import '../../../widgets/tile/cover_art.dart';
import '../../../widgets/track_list/utils/internal_media_file.dart';
import '../../../bindings/bindings.dart';

import './search_card.dart';

class TrackSearchItem extends SearchCard {
  final InternalMediaFile item;
  final List<int> fallbackFileIds;
  const TrackSearchItem({
    super.key,
    required super.index,
    required this.item,
    required this.fallbackFileIds,
  });

  @override
  TrackSearchItemState createState() => TrackSearchItemState();
}

class TrackSearchItemState extends SearchCardState<TrackSearchItem> {
  @override
  int getItemId() => widget.item.id;

  @override
  String getItemTitle() => widget.item.title;

  @override
  Widget buildLeadingWidget(double size) {
    return CoverArt(
      path: widget.item.coverArtPath,
      hint: (
        widget.item.album,
        widget.item.artist,
        'Total Time ${formatTime(widget.item.duration)}'
      ),
      size: size,
    );
  }

  @override
  void onPressed(BuildContext context) {
    operatePlaybackWithMixQuery(
      queries: const QueryList([]),
      playbackMode: 99,
      hintPosition: 0,
      initialPlaybackId: widget.item.id,
      operateMode: PlaylistOperateMode.replace,
      instantlyPlay: true,
      fallbackPlayingItems:
          widget.fallbackFileIds.map(PlayingItem.inLibrary).toList(),
    );
  }

  @override
  void onContextMenu(BuildContext context, Offset position) {
    openTrackItemContextMenu(
      position,
      context,
      contextAttachKey,
      contextController,
      null,
      widget.item.id,
      null,
      null,
    );
  }

  @override
  void onMiddleClick(BuildContext context, Offset position) {
    executeMiddleClickAction(
      context,
      CollectionType.track,
      widget.item.id,
    );
  }
}
