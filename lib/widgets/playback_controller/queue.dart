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
import '../capability_gate.dart';
import '../../utils/dialogs/information/error.dart';
import '../../utils/api/request_failure.dart';

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

class PlaylistQueueItem extends StatelessWidget {
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
  Widget build(BuildContext context) {
    final typography = FluentTheme.of(context).typography;
    final accentColor = Color.alphaBlend(
      FluentTheme.of(context).inactiveColor.withAlpha(100),
      FluentTheme.of(context).accentColor,
    );
    final reason = _unavailableReason(context, entry.entry.unavailable);
    final color = isCurrent ? accentColor : null;
    final moveCapability = capabilityForRequest(
      context,
      (capabilities) => capabilities.movePlaylistItemRequest,
    );

    // The queue stays visible on any target; only switching to an item is gated.
    final canSwitch =
        capabilityForRequest(
              context,
              (capabilities) => capabilities.switchRequest,
            )
            is RequestCapabilityAvailable;

    return ListTile.selectable(
      title: Transform.translate(
        offset: const Offset(-8, 0),
        child: Row(
          children: [
            isCurrent
                ? Icon(Symbols.play_arrow, color: color, size: 24)
                : const SizedBox(width: 24),
            const SizedBox(width: 4),
            SizedBox(
              width: 320,
              child: Opacity(
                opacity: reason == null ? 1 : 0.5,
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      entry.entry.title,
                      overflow: TextOverflow.ellipsis,
                      style: typography.body?.apply(color: color),
                    ),
                    Opacity(
                      opacity: isCurrent ? 0.8 : 0.46,
                      child: Text(
                        entry.entry.artist,
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
            ),
          ],
        ),
      ),
      trailing: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          if (moveCapability case RequestCapabilityDisabled(:final reason))
            Tooltip(
              message: capabilityReasonText(context, reason),
              child: const Padding(
                padding: EdgeInsets.all(8),
                child: Icon(FluentIcons.info),
              ),
            ),
          CapabilityGate(
            capability: capabilityForRequest(
              context,
              (capabilities) => capabilities.removeRequest,
            ),
            child: IconButton(
              icon: const Icon(Symbols.close),
              onPressed: () =>
                  context.read<PlaylistProvider>().removeItem(entry.index),
            ),
          ),
        ],
      ),
      onPressed: reason == null
          ? (canSwitch ? onPlay : null)
          : () => showErrorDialog(
              context: context,
              title: S.of(context).unavailable,
              subtitle: entry.entry.title,
              errorMessage: reason,
            ),
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
                          delegate: ReorderableSliverChildBuilderDelegate((
                            BuildContext context,
                            int index,
                          ) {
                            final x = playlistProvider.items[index];
                            final isCurrent =
                                playbackStatusProvider.$1 == x.index &&
                                playbackStatusProvider.$2 == x.item;
                            return PlaylistQueueItem(
                              key: ValueKey(x.entry.item),
                              entry: x,
                              isCurrent: isCurrent,
                              onPlay: () => playlistProvider.playItem(x.index),
                            );
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
                            return PlaylistQueueItem(
                              key: ValueKey(x.entry.item),
                              entry: x,
                              isCurrent: isCurrent,
                              onPlay: () => playlistProvider.playItem(x.index),
                            );
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
