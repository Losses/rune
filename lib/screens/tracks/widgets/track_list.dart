import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:infinite_scroll_pagination/infinite_scroll_pagination.dart';
import 'package:player/widgets/cover_art.dart';

import '../../../utils/platform.dart';
import '../../../messages/playback.pb.dart';
import '../../../messages/media_file.pb.dart';
import '../../../messages/recommend.pbserver.dart';

class TrackListView extends StatefulWidget {
  const TrackListView({super.key});

  @override
  TrackListViewState createState() => TrackListViewState();
}

class TrackListViewState extends State<TrackListView> {
  static const _pageSize = 20;

  final PagingController<int, MediaFile> _pagingController =
      PagingController(firstPageKey: 0);

  @override
  void initState() {
    _pagingController.addPageRequestListener((cursor) {
      _fetchPage(cursor);
    });
    super.initState();
  }

  Future<void> _fetchPage(int cursor) async {
    try {
      final fetchMediaFiles = FetchMediaFiles(
        cursor: cursor,
        pageSize: _pageSize,
      );
      fetchMediaFiles.sendSignalToRust(); // GENERATED

      // Listen for the response from Rust
      final rustSignal = await MediaFileList.rustSignalStream.first;
      final mediaFileList = rustSignal.message;
      final newItems = mediaFileList.mediaFiles;

      final isLastPage = newItems.length < _pageSize;
      if (isLastPage) {
        _pagingController.appendLastPage(newItems);
      } else {
        final nextCursor = cursor + newItems.length;
        _pagingController.appendPage(newItems, nextCursor);
      }
    } catch (error) {
      _pagingController.error = error;
    }
  }

  @override
  Widget build(BuildContext context) {
    return PagedListView<int, MediaFile>(
      pagingController: _pagingController,
      builderDelegate: PagedChildBuilderDelegate<MediaFile>(
        itemBuilder: (context, item, index) => TrackListItem(
          index: index,
          item: item,
        ),
      ),
    );
  }

  @override
  void dispose() {
    _pagingController.dispose();
    super.dispose();
  }
}

class TrackListItem extends StatelessWidget {
  final MediaFile item;
  final int index;

  final contextController = FlyoutController();
  final contextAttachKey = GlobalKey();

  TrackListItem({
    super.key,
    required this.index,
    required this.item,
  });

  openContextMenu(Offset localPosition, BuildContext context) {
    final targetContext = contextAttachKey.currentContext;

    if (targetContext == null) return;
    final box = targetContext.findRenderObject() as RenderBox;
    final position = box.localToGlobal(
      localPosition,
      ancestor: Navigator.of(context).context.findRenderObject(),
    );

    contextController.showFlyout(
      barrierColor: Colors.black.withOpacity(0.1),
      position: position,
      builder: (context) {
        var items = [
          MenuFlyoutItem(
            leading: const Icon(Symbols.rocket),
            text: const Text('Roaming'),
            onPressed: () => {
              RecommendAndPlayRequest(fileId: item.id)
                  .sendSignalToRust() // GENERATED
            },
          ),
        ];

        return MenuFlyout(
          items: items,
        );
      },
    );
  }

  @override
  Widget build(BuildContext context) {
    Typography typography = FluentTheme.of(context).typography;

    return GestureDetector(
        onSecondaryTapUp: isDesktop
            ? (d) {
                openContextMenu(d.localPosition, context);
              }
            : null,
        onLongPressEnd: isDesktop
            ? null
            : (d) {
                openContextMenu(d.localPosition, context);
              },
        child: FlyoutTarget(
            key: contextAttachKey,
            controller: contextController,
            child: ListTile.selectable(
                title: Row(
                  children: [
                    // SizedBox(
                    //     width: 52,
                    //     child: Text(
                    //       (index + 1).toString(),
                    //       style:
                    //           typography.bodyLarge?.apply(fontSizeFactor: 1.2),
                    //       textAlign: TextAlign.right,
                    //     )),
                    // const SizedBox(width: 8),
                    CoverArt(fileId: item.id, size: 48),
                    const SizedBox(width: 12),
                    Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          item.title,
                          overflow: TextOverflow.ellipsis,
                        ),
                        const SizedBox(height: 8),
                        Opacity(
                          opacity: 0.46,
                          child: Text(
                            item.artist,
                            style: typography.caption,
                            overflow: TextOverflow.ellipsis,
                          ),
                        ),
                      ],
                    )
                  ],
                ),
                onSelectionChange: (v) => PlayFileRequest(fileId: item.id)
                    .sendSignalToRust() // GENERATED,
                )));
  }
}
