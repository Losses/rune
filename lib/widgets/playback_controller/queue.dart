import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:reorderables/reorderables.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/l10n.dart';
import '../../utils/playing_item.dart';
import '../../bindings/bindings.dart';
import '../../providers/playlist.dart';
import '../../providers/status.dart';

class Queue extends StatefulWidget {
  const Queue({super.key});

  @override
  State<Queue> createState() => _QueueState();
}

class _QueueState extends State<Queue> {
  final ScrollController _scrollController = ScrollController();

  @override
  void dispose() {
    super.dispose();

    _scrollController.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Selector<PlaybackStatusProvider, (int?, PlayingItem?)>(
      selector: (context, playbackStatusProvider) => (
        playbackStatusProvider.playbackStatus.index,
        playbackStatusProvider.playingItem
      ),
      builder: (context, playbackStatusProvider, child) {
        Typography typography = FluentTheme.of(context).typography;
        Color accentColor = Color.alphaBlend(
          FluentTheme.of(context).inactiveColor.withAlpha(100),
          FluentTheme.of(context).accentColor,
        );

        return Consumer<PlaylistProvider>(
          builder: (context, playlistProvider, child) {
            void onReorder(int oldIndex, int newIndex) {
              playlistProvider.reorderItems(oldIndex, newIndex);
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
                      ReorderableSliverList(
                        onReorder: onReorder,
                        delegate: ReorderableSliverChildBuilderDelegate(
                          (BuildContext context, int index) {
                            final x = playlistProvider.items[index];
                            final isCurrent =
                                playbackStatusProvider.$1 == x.index &&
                                    playbackStatusProvider.$2 == x.item;
                            final color = isCurrent ? accentColor : null;

                            return ListTile.selectable(
                              key: ValueKey(x.entry.item),
                              title: Transform.translate(
                                offset: const Offset(-8, 0),
                                child: Row(
                                  children: [
                                    isCurrent
                                        ? Icon(
                                            Symbols.play_arrow,
                                            color: color,
                                            size: 24,
                                          )
                                        : const SizedBox(width: 24),
                                    const SizedBox(width: 4),
                                    SizedBox(
                                      width: 320,
                                      child: Column(
                                        crossAxisAlignment:
                                            CrossAxisAlignment.start,
                                        children: [
                                          Text(
                                            x.entry.title,
                                            overflow: TextOverflow.ellipsis,
                                            style: typography.body
                                                ?.apply(color: color),
                                          ),
                                          Opacity(
                                            opacity: isCurrent ? 0.8 : 0.46,
                                            child: Text(
                                              x.entry.artist,
                                              overflow: TextOverflow.ellipsis,
                                              style: typography.caption
                                                  ?.apply(color: color),
                                            ),
                                          ),
                                        ],
                                      ),
                                    )
                                  ],
                                ),
                              ),
                              onPressed: () => SwitchRequest(index: x.index)
                                  .sendSignalToRust(),
                            );
                          },
                          childCount: playlistProvider.items.length,
                        ),
                      ),
                    ],
                  );
          },
        );
      },
    );
  }
}
