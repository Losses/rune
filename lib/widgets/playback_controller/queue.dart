import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:reorderables/reorderables.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../generated/l10n.dart';
import '../../messages/playback.pb.dart';
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
    return Selector<PlaybackStatusProvider, (int?, int?)>(
      selector: (context, playbackStatusProvider) => (
        playbackStatusProvider.playbackStatus?.index,
        playbackStatusProvider.playbackStatus?.id
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
                            final item = playlistProvider.items[index];
                            final isCurrent =
                                playbackStatusProvider.$1 == item.index &&
                                    playbackStatusProvider.$2 == item.entry.id;
                            final color = isCurrent ? accentColor : null;

                            return ListTile.selectable(
                              key: ValueKey(item.entry.id),
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
                                            item.entry.title,
                                            overflow: TextOverflow.ellipsis,
                                            style: typography.body
                                                ?.apply(color: color),
                                          ),
                                          Opacity(
                                            opacity: isCurrent ? 0.8 : 0.46,
                                            child: Text(
                                              item.entry.artist,
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
                              onPressed: () => SwitchRequest(index: item.index)
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
