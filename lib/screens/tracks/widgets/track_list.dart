import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:infinite_scroll_pagination/infinite_scroll_pagination.dart';

import '../../../config/animation.dart';
import '../../../widgets/track_list/track_list.dart';
import '../../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../../messages/media_file.pb.dart';

class TrackListView extends StatefulWidget {
  final StartScreenLayoutManager layoutManager;

  const TrackListView({
    super.key,
    required this.layoutManager,
  });

  @override
  TrackListViewState createState() => TrackListViewState();
}

class TrackListViewState extends State<TrackListView> {
  static const _pageSize = 100;

  final PagingController<int, MediaFile> _pagingController =
      PagingController(firstPageKey: 0);

  @override
  void initState() {
    super.initState();
    _pagingController.addPageRequestListener((cursor) async {
      _fetchPage(cursor);
    });
  }

  Future<void> _fetchPage(int cursor) async {
    try {
      final fetchMediaFiles = FetchMediaFilesRequest(
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

      Timer(Duration(milliseconds: gridAnimationDelay),
          () => widget.layoutManager.playAnimations());
    } catch (error) {
      _pagingController.error = error;
    }
  }

  @override
  Widget build(BuildContext context) {
    return TrackList(
      pagingController: _pagingController,
      queries: const [("lib::all", "true")],
      mode: 99,
    );
  }

  @override
  void dispose() {
    _pagingController.dispose();
    super.dispose();
  }
}
