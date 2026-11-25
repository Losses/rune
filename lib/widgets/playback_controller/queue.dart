import 'dart:async';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:reorderables/reorderables.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/l10n.dart';
import '../../utils/playing_item.dart';
import '../../bindings/bindings.dart';
import '../../providers/playlist.dart';
import '../../providers/status.dart';
import '../../providers/request_capabilities.dart';
import '../../utils/dialogs/information/error.dart';
import '../../utils/api/request_failure.dart';
import '../../utils/router/router_aware_flyout_controller.dart';
import '../context_menu_wrapper.dart';

String? _unavailableReason(
  BuildContext context,
  PlaylistUnavailableReason? reason,
) {
  final s = S.of(context);
  return switch (reason) {
    PlaylistUnavailableReason.accessRevoked => s.queueUnavailableAccessRevoked,
    PlaylistUnavailableReason.libraryUnavailable =>
      s.queueUnavailableLibraryUnavailable,
    PlaylistUnavailableReason.sourceUnreachable =>
      s.queueUnavailableSourceUnreachable,
    PlaylistUnavailableReason.unreadable => s.queueUnavailableUnreadable,
    null => null,
  };
}

class PlaylistQueueItem extends StatefulWidget {
  const PlaylistQueueItem({
    super.key,
    required this.entry,
    required this.isCurrent,
    required this.onPlay,
  });

  final PlaylistEntry entry;
  final bool isCurrent;
  final VoidCallback onPlay;

  @override
  State<PlaylistQueueItem> createState() => _PlaylistQueueItemState();
}

class _PlaylistQueueItemState extends State<PlaylistQueueItem> {
  final _contextController = RouterAwareFlyoutController();
  final _contextAttachKey = GlobalKey();

  @override
  void dispose() {
    _contextController.dispose();
    super.dispose();
  }

  void _showContextMenu(Offset localPosition) {
    final targetContext = _contextAttachKey.currentContext;
    if (targetContext == null) return;

    final box = targetContext.findRenderObject() as RenderBox;
    final position = box.localToGlobal(
      localPosition,
      ancestor: Navigator.of(context).context.findRenderObject(),
    );
    final removeCapability = capabilityForRequest(
      context,
      (capabilities) => capabilities.removeRequest,
      listen: false,
    );
    if (removeCapability is! RequestCapabilityAvailable) return;

    _contextController.showFlyout(
      position: position,
      builder: (context) => MenuFlyout(
        items: [
          MenuFlyoutItem(
            leading: const Icon(Symbols.delete),
            text: Text(S.of(context).removeFromPlaylist),
            onPressed: () {
              Navigator.pop(context);
              this.context.read<PlaylistProvider>().removeItem(
                widget.entry.index,
              );
            },
          ),
        ],
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final typography = FluentTheme.of(context).typography;
    final accentColor = Color.alphaBlend(
      FluentTheme.of(context).inactiveColor.withAlpha(100),
      FluentTheme.of(context).accentColor,
    );
    final reason = _unavailableReason(context, widget.entry.entry.unavailable);
    final color = widget.isCurrent ? accentColor : null;

    // The queue stays visible on any target; only switching to an item is gated.
    final canSwitch =
        capabilityForRequest(
              context,
              (capabilities) => capabilities.switchRequest,
            )
            is RequestCapabilityAvailable;

    final tile = ListTile.selectable(
      title: Transform.translate(
        offset: const Offset(-8, 0),
        child: Row(
          children: [
            widget.isCurrent
                ? Icon(Symbols.play_arrow, color: color, size: 24)
                : const SizedBox(width: 24),
            const SizedBox(width: 4),
            Flexible(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    widget.entry.entry.title,
                    overflow: TextOverflow.ellipsis,
                    style: typography.body?.apply(color: color),
                  ),
                  Opacity(
                    opacity: widget.isCurrent ? 0.8 : 0.46,
                    child: Text(
                      widget.entry.entry.artist,
                      overflow: TextOverflow.ellipsis,
                      style: typography.caption?.apply(color: color),
                    ),
                  ),
                  if (reason != null)
                    Text(
                      reason,
                      key: const Key('queue-unavailable-reason'),
                      overflow: TextOverflow.ellipsis,
                      style: typography.caption?.apply(color: color),
                    ),
                ],
              ),
            ),
          ],
        ),
      ),
      onPressed: reason == null
          ? (canSwitch ? widget.onPlay : null)
          : () => showErrorDialog(
              context: context,
              title: S.of(context).unavailable,
              subtitle: widget.entry.entry.title,
              errorMessage: reason,
            ),
    );

    return ContextMenuWrapper(
      contextAttachKey: _contextAttachKey,
      contextController: _contextController,
      onContextMenu: _showContextMenu,
      onMiddleClick: (_) {},
      child: tile,
    );
  }
}

class Queue extends StatefulWidget {
  const Queue({super.key});

  @override
  State<Queue> createState() => _QueueState();
}

class _QueueState extends State<Queue> {
  final ScrollController _scrollController = ScrollController();
  late final StreamSubscription<RequestFailure> _failureSubscription;
  static const _revealAttempts = 8;
  bool _revealScheduled = false;
  // Held by the playing row only, so the reveal reaches it without searching.
  // The reorderable list builds a dragged row's feedback, the placeholder it
  // leaves and the gap it would drop into from the same widget, so a key that
  // stayed on the row would be mounted three times once the row is dragged.
  // The row therefore holds the key only while the reveal runs, a few frames
  // after the flyout opens, and dragging is off until then.
  final _currentRowKey = GlobalKey();
  bool _revealing = false;

  @override
  void initState() {
    super.initState();
    _failureSubscription = context
        .read<PlaylistProvider>()
        .failureFeedback
        .listen((failure) {
          if (!mounted) return;
          showErrorDialog(
            context: context,
            title: S.of(context).unavailable,
            errorMessage: requestFailureMessage(S.of(context), failure),
          );
        });
  }

  @override
  void dispose() {
    super.dispose();

    _scrollController.dispose();
    _failureSubscription.cancel();
  }

  // Runs once, on the first build that identifies the playing row, so the row
  // is in view when the flyout opens. A build before the playback status
  // arrives has no playing row yet, so it must not use up the one run. The
  // rows build lazily and may differ in height, so the list jumps to an
  // estimate from the average row height, then the playing row, once built, is
  // placed exactly.
  void _revealCurrent(int index, int count) {
    if (_revealScheduled || index < 0) return;
    _revealScheduled = true;
    if (index == 0) return;
    _revealing = true;
    _jumpTowardsCurrent(index, count, _revealAttempts);
  }

  // Runs [stage] at the end of a frame. A post-frame callback does not ask for
  // a frame by itself, so one added after a frame was built, as every stage
  // after the first is, would wait until something else changed on screen.
  void _afterFrame(VoidCallback stage) {
    WidgetsBinding.instance.addPostFrameCallback((_) => stage());
    WidgetsBinding.instance.ensureVisualUpdate();
  }

  // The estimate improves with every jump because the rows built on the way
  // raise or lower the average, so a few jumps reach a row the first one missed.
  void _jumpTowardsCurrent(int index, int count, int attemptsLeft) {
    _afterFrame(() {
      if (!mounted) return;
      if (!_scrollController.hasClients) {
        setState(() => _revealing = false);
        return;
      }
      final position = _scrollController.position;
      final rowExtent =
          (position.maxScrollExtent + position.viewportDimension) / count;
      position.jumpTo(
        ((index - 2) * rowExtent).clamp(0.0, position.maxScrollExtent),
      );
      _afterFrame(() {
        if (!mounted) return;
        final rowContext = _currentRowKey.currentContext;
        if (rowContext == null && attemptsLeft > 1) {
          _jumpTowardsCurrent(index, count, attemptsLeft - 1);
          return;
        }
        if (rowContext != null) {
          Scrollable.ensureVisible(rowContext, alignment: 0.2);
        }
        setState(() => _revealing = false);
      });
    });
  }

  @override
  Widget build(BuildContext context) {
    return Selector<PlaybackStatusProvider, (int?, PlayingItem?)>(
      selector: (context, playbackStatusProvider) => (
        playbackStatusProvider.playbackStatus.index,
        playbackStatusProvider.playingItem,
      ),
      builder: (context, playbackStatusProvider, child) {
        return Consumer<PlaylistProvider>(
          builder: (context, playlistProvider, child) {
            void onReorder(int oldIndex, int newIndex) {
              playlistProvider.reorderItems(oldIndex, newIndex);
            }

            final moveCapability = capabilityForRequest(
              context,
              (capabilities) => capabilities.movePlaylistItemRequest,
            );
            final canMove = moveCapability is RequestCapabilityAvailable;

            if (playlistProvider.items.isNotEmpty) {
              _revealCurrent(
                playlistProvider.items.indexWhere(
                  (x) =>
                      playbackStatusProvider.$1 == x.index &&
                      playbackStatusProvider.$2 == x.item,
                ),
                playlistProvider.items.length,
              );
            }

            return playlistProvider.items.isEmpty
                ? ListTile.selectable(
                    key: const Key("disabled"),
                    leading: const Icon(Symbols.info),
                    title: Text(S.of(context).noItemsInPlaylist),
                    onPressed: () {},
                  )
                : CustomScrollView(
                    controller: _scrollController,
                    slivers: [
                      if (canMove)
                        ReorderableSliverList(
                          onReorder: onReorder,
                          enabled: !_revealing,
                          delegate: ReorderableSliverChildBuilderDelegate((
                            BuildContext context,
                            int index,
                          ) {
                            final x = playlistProvider.items[index];
                            final isCurrent =
                                playbackStatusProvider.$1 == x.index &&
                                playbackStatusProvider.$2 == x.item;
                            final row = PlaylistQueueItem(
                              key: ValueKey(x.entry.item),
                              entry: x,
                              isCurrent: isCurrent,
                              onPlay: () => playlistProvider.playItem(x.index),
                            );
                            return isCurrent && _revealing
                                ? KeyedSubtree(key: _currentRowKey, child: row)
                                : row;
                          }, childCount: playlistProvider.items.length),
                        )
                      else
                        SliverList.builder(
                          itemCount: playlistProvider.items.length,
                          itemBuilder: (context, index) {
                            final x = playlistProvider.items[index];
                            final isCurrent =
                                playbackStatusProvider.$1 == x.index &&
                                playbackStatusProvider.$2 == x.item;
                            final row = PlaylistQueueItem(
                              key: ValueKey(x.entry.item),
                              entry: x,
                              isCurrent: isCurrent,
                              onPlay: () => playlistProvider.playItem(x.index),
                            );
                            return isCurrent && _revealing
                                ? KeyedSubtree(key: _currentRowKey, child: row)
                                : row;
                          },
                        ),
                    ],
                  );
          },
        );
      },
    );
  }
}
