import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/execute_middle_click_action.dart';
import '../../../utils/query_list.dart';
import '../../../utils/format_time.dart';
import '../../../utils/api/operate_playback_with_mix_query.dart';
import '../../../utils/context_menu/track_item_context_menu.dart';
import '../../../widgets/tile/cover_art.dart';
import '../../../widgets/track_list/utils/internal_media_file.dart';
import '../../../messages/all.dart';

import './search_card.dart';

class TrackSearchItem extends SearchCard {
  final InternalMediaFile item;
  final List<int> fallbackFileIds;

  TrackSearchItem({
    super.key,
    required super.index,
    required this.item,
    required this.fallbackFileIds,
  });

  @override
  int getItemId() => item.id;

  @override
  String getItemTitle() => item.title;

  @override
  Widget buildLeadingWidget(double size) {
    return CoverArt(
      path: item.coverArtPath,
      hint: (
        item.album,
        item.artist,
        'Total Time ${formatTime(item.duration)}'
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
      initialPlaybackId: item.id,
      operateMode: PlaylistOperateMode.Replace,
      instantlyPlay: true,
      fallbackFileIds: fallbackFileIds,
    );
  }

  @override
  void onContextMenu(BuildContext context, Offset position) {
    openTrackItemContextMenu(
      position,
      context,
      contextAttachKey,
      contextController,
      item.id,
    );
  }

  @override
  void onMiddleClick(BuildContext context, Offset position) {
    executeMiddleClickAction(
      context,
      CollectionType.Track,
      item.id,
    );
  }
}
