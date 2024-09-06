import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:infinite_scroll_pagination/infinite_scroll_pagination.dart';

import '../../../config/animation.dart';
import '../../../widgets/track_list/track_list.dart';
import '../../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../../messages/media_file.pb.dart';

class QueryTrackListView extends StatefulWidget {
  final List<int> artistIds;
  final List<int> albumIds;
  final List<int> playlistIds;
  final StartScreenLayoutManager layoutManager;

  const QueryTrackListView({
    super.key,
    required this.layoutManager,
    this.artistIds = const [],
    this.albumIds = const [],
    this.playlistIds = const [],
  });

  @override
  QueryTrackListViewState createState() => QueryTrackListViewState();
}

class QueryTrackListViewState extends State<QueryTrackListView> {
  static const _pageSize = 20;

  final PagingController<int, MediaFile> _pagingController =
      PagingController(firstPageKey: 0);

  @override
  void initState() {
    _pagingController.addPageRequestListener((cursor) async {
      await _fetchPage(cursor);

      Timer(Duration(milliseconds: gridAnimationDelay),
          () => widget.layoutManager.playAnimations());
    });
    super.initState();
  }

  Future<void> _fetchPage(int cursor) async {
    try {
      final fetchMediaFiles = CompoundQueryMediaFilesRequest(
        cursor: cursor,
        pageSize: _pageSize,
        artistIds: widget.artistIds,
        albumIds: widget.albumIds,
        playlistIds: widget.playlistIds,
      );
      fetchMediaFiles.sendSignalToRust(); // GENERATED

      // Listen for the response from Rust
      final rustSignal =
          await CompoundQueryMediaFilesResponse.rustSignalStream.first;
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
    return TrackList(pagingController: _pagingController);
  }

  @override
  void dispose() {
    _pagingController.dispose();
    super.dispose();
  }
}
