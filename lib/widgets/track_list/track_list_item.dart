import 'dart:math';
import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/context_menu/track_item_context_menu.dart';
import '../../widgets/cover_art.dart';
import '../../widgets/context_menu_wrapper.dart';
import '../../messages/mix.pb.dart';
import '../../messages/playback.pb.dart';
import '../../messages/media_file.pb.dart';

class TrackListItem extends StatelessWidget {
  final MediaFile item;
  final int index;
  final List<(String, String)> queries;
  final int mode;

  final contextController = FlyoutController();
  final contextAttachKey = GlobalKey();

  TrackListItem({
    super.key,
    required this.index,
    required this.item,
    required this.queries,
    required this.mode,
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
        onPressed: () {
          OperatePlaybackWithMixQueryRequest(
            queries: queries
                .map((x) => MixQuery(operator: x.$1, parameter: x.$2))
                .toList(),
            playbackMode: mode,
            hintPosition: index,
            initialPlaybackId: item.id,
            replacePlaylist: true,
            instantlyPlay: true,
          ).sendSignalToRust();
        },
        child: ClipRRect(
          borderRadius: BorderRadius.circular(3),
          child: LayoutBuilder(
            builder: (context, constraints) {
              final size = min(constraints.maxWidth, constraints.maxHeight);
              return Row(
                children: [
                  CoverArt(
                    fileId: item.id,
                    size: size,
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
