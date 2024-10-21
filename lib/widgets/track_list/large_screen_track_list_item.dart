import 'dart:math';
import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/query_list.dart';
import '../../utils/format_time.dart';
import '../../utils/api/operate_playback_with_mix_query.dart';
import '../../utils/context_menu/track_item_context_menu.dart';
import '../../widgets/context_menu_wrapper.dart';
import '../../widgets/track_list/utils/internal_media_file.dart';
import '../../messages/playback.pb.dart';

import '../tile/cover_art.dart';

class LargeScreenTrackListItem extends StatelessWidget {
  final InternalMediaFile item;
  final int index;
  final QueryList queries;
  final int mode;
  final String? coverArtPath;
  final List<int> fallbackFileIds;

  final contextController = FlyoutController();
  final contextAttachKey = GlobalKey();

  LargeScreenTrackListItem({
    super.key,
    required this.index,
    required this.item,
    required this.queries,
    required this.mode,
    required this.fallbackFileIds,
    required this.coverArtPath,
  });

  @override
  Widget build(BuildContext context) {
    Typography typography = FluentTheme.of(context).typography;

    return ContextMenuWrapper(
      contextAttachKey: contextAttachKey,
      contextController: contextController,
      onContextMenu: (position) {
        openTrackItemContextMenu(
            position, context, contextAttachKey, contextController, item.id);
      },
      child: Button(
        style: const ButtonStyle(
          padding: WidgetStatePropertyAll(EdgeInsets.all(0)),
        ),
        onPressed: () async {
          if (context.mounted) {
            await safeOperatePlaybackWithMixQuery(
              context: context,
              queries: queries,
              playbackMode: mode,
              hintPosition: index,
              initialPlaybackId: item.id,
              operateMode: PlaylistOperateMode.Replace,
              instantlyPlay: true,
              fallbackFileIds: fallbackFileIds,
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
                  CoverArt(
                    path: coverArtPath,
                    size: size,
                    hint: (
                      item.album,
                      item.artist,
                      'Total Time ${formatTime(item.duration)}'
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
                            item.title,
                            overflow: TextOverflow.ellipsis,
                          ),
                          const SizedBox(height: 4),
                          Text(
                            item.artist,
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
