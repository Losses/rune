import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/query_list.dart';
import '../../utils/format_time.dart';
import '../../utils/api/operate_playback_with_mix_query.dart';
import '../../utils/context_menu/track_item_context_menu.dart';
import '../../widgets/context_menu_wrapper.dart';
import '../../widgets/track_list/utils/internal_media_file.dart';

import '../tile/cover_art.dart';

class SmallScreenTrackListItem extends StatefulWidget {
  final InternalMediaFile item;
  final int index;
  final QueryList queries;
  final int mode;
  final String? coverArtPath;
  final List<int> fallbackFileIds;

  const SmallScreenTrackListItem({
    super.key,
    required this.index,
    required this.item,
    required this.queries,
    required this.mode,
    required this.fallbackFileIds,
    required this.coverArtPath,
  });

  @override
  State<SmallScreenTrackListItem> createState() =>
      _SmallScreenTrackListItemState();
}

class _SmallScreenTrackListItemState extends State<SmallScreenTrackListItem> {
  final contextController = FlyoutController();
  final contextAttachKey = GlobalKey();

  bool _isHovered = false;
  final FocusNode _focusNode = FocusNode();

  void onPressed() async {
    if (!context.mounted) return;

    await safeOperatePlaybackWithMixQuery(
      context: context,
      queries: widget.queries,
      playbackMode: widget.mode,
      hintPosition: widget.index,
      initialPlaybackId: widget.item.id,
      replacePlaylist: true,
      instantlyPlay: true,
      fallbackFileIds: widget.fallbackFileIds,
    );
  }

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final typography = theme.typography;

    return ContextMenuWrapper(
      contextAttachKey: contextAttachKey,
      contextController: contextController,
      onContextMenu: (position) {
        openTrackItemContextMenu(
          position,
          context,
          contextAttachKey,
          contextController,
          widget.item.id,
        );
      },
      child: GestureDetector(
        onTap: onPressed,
        child: MouseRegion(
          onEnter: (_) => setState(() => _isHovered = true),
          onExit: (_) => setState(() => _isHovered = false),
          child: FocusableActionDetector(
            focusNode: _focusNode,
            child: Padding(
              padding: const EdgeInsets.symmetric(vertical: 1),
              child: Row(
                children: [
                  SizedBox(
                    width: 40,
                    height: 40,
                    child: CoverArt(
                      path: widget.coverArtPath,
                      size: 40,
                      hint: (
                        widget.item.album,
                        widget.item.artist,
                        'Total Time ${formatTime(widget.item.duration)}'
                      ),
                    ),
                  ),
                  Expanded(
                    child: Padding(
                      padding: const EdgeInsets.all(8),
                      child: AnimatedOpacity(
                        opacity: _isHovered ? 1.0 : 0.8,
                        duration: theme.fastAnimationDuration,
                        child: Column(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          mainAxisAlignment: MainAxisAlignment.center,
                          children: [
                            Text(
                              widget.item.title,
                              style: typography.bodyLarge?.apply(),
                              overflow: TextOverflow.ellipsis,
                            ),
                            const SizedBox(height: 1),
                            Text(
                              widget.item.artist,
                              style: typography.caption?.apply(
                                color:
                                    typography.caption?.color?.withAlpha(117),
                              ),
                              overflow: TextOverflow.ellipsis,
                            ),
                          ],
                        ),
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}
