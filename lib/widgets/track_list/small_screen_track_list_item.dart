import 'dart:ui';

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
import '../../widgets/navigation_bar/utils/activate_link_action.dart';
import '../../bindings/bindings.dart';

import '../tile/cover_art.dart';
import '../ax_reveal/ax_reveal.dart';

class SmallScreenTrackListItem extends StatefulWidget {
  final InternalMediaFile item;
  final int index;
  final QueryList queries;
  final int mode;
  final String? coverArtPath;
  final List<int> fallbackFileIds;
  final int position;
  final void Function()? reloadData;

  const SmallScreenTrackListItem({
    super.key,
    required this.index,
    required this.item,
    required this.queries,
    required this.mode,
    required this.fallbackFileIds,
    required this.coverArtPath,
    required this.position,
    required this.reloadData,
  });

  @override
  State<SmallScreenTrackListItem> createState() =>
      _SmallScreenTrackListItemState();
}

class _SmallScreenTrackListItemState extends State<SmallScreenTrackListItem> {
  final _contextController = RouterAwareFlyoutController();
  final _contextAttachKey = GlobalKey();

  bool _isHovered = false;
  bool _isFocused = false;

  void _handleFocusHighlight(bool value) {
    setState(() {
      _isFocused = value;
    });
  }

  void _handleHoverHighlight(bool value) {
    setState(() {
      _isHovered = value;
    });
  }

  void onPressed() async {
    if (!context.mounted) return;

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

  @override
  void dispose() {
    super.dispose();
    _contextController.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final typography = theme.typography;

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
      child: GestureDetector(
        onTap: onPressed,
        child: FocusableActionDetector(
          onShowFocusHighlight: _handleFocusHighlight,
          onShowHoverHighlight: _handleHoverHighlight,
          actions: {
            ActivateIntent: ActivateLinkAction(context, onPressed),
          },
          child: TweenAnimationBuilder<double>(
            duration: theme.fastAnimationDuration,
            tween: Tween<double>(begin: 0, end: _isFocused ? 1 : 0),
            builder: (context, focusValue, child) {
              final contentColor = Color.lerp(
                theme.typography.title!.color!,
                theme.brightness == Brightness.dark
                    ? theme.accentColor.lighter
                    : theme.accentColor.darker,
                focusValue,
              )!;

              final shadowColor = Color.lerp(
                theme.typography.title!.color!,
                theme.brightness == Brightness.dark
                    ? theme.accentColor.darker
                    : theme.accentColor.lighter,
                focusValue,
              )!;

              return TweenAnimationBuilder<double>(
                duration: theme.fastAnimationDuration,
                tween: Tween<double>(
                    begin: 0, end: _isHovered || _isFocused ? 1 : 0),
                builder: (context, hoverValue, child) {
                  final titleAlpha = lerpDouble(204, 255, hoverValue)!.toInt();
                  final subtitleAlpha =
                      lerpDouble(94, 117, hoverValue)!.toInt();

                  final List<Shadow> textShadows = [
                    Shadow(
                      color: shadowColor,
                      blurRadius: focusValue * 8,
                    ),
                  ];

                  return Padding(
                    padding: const EdgeInsets.symmetric(vertical: 1),
                    child: Row(
                      children: [
                        Container(
                          width: 40,
                          height: 40,
                          decoration: BoxDecoration(
                            border: Border.all(
                              color: focusValue == 0.0
                                  ? Colors.transparent
                                  : theme.accentColor,
                              width: focusValue * 2,
                            ),
                            boxShadow: [
                              BoxShadow(
                                color: theme.accentColor
                                    .withValues(alpha: 0.5 * focusValue),
                                blurRadius: focusValue * 4,
                                spreadRadius: focusValue * 2,
                              ),
                            ],
                          ),
                          child: AxReveal0(
                            child: ClipRRect(
                              borderRadius: BorderRadius.circular(4),
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
                                  style: typography.bodyLarge?.apply(
                                    color: contentColor.withAlpha(titleAlpha),
                                    shadows: textShadows,
                                  ),
                                  overflow: TextOverflow.ellipsis,
                                ),
                                const SizedBox(height: 1),
                                Text(
                                  widget.item.artist,
                                  style: typography.caption?.apply(
                                    color:
                                        contentColor.withAlpha(subtitleAlpha),
                                  ),
                                  overflow: TextOverflow.ellipsis,
                                ),
                              ],
                            ),
                          ),
                        ),
                      ],
                    ),
                  );
                },
              );
            },
          ),
        ),
      ),
    );
  }
}
