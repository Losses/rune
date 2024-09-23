import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:infinite_scroll_pagination/infinite_scroll_pagination.dart';

import '../../../utils/api/query_mix_tracks.dart';
import '../../../config/animation.dart';
import '../../../widgets/track_list/track_list.dart';
import '../../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../../messages/media_file.pb.dart';

class QueryTrackListView extends StatefulWidget {
  final List<(String, String)> queries;
  final StartScreenLayoutManager layoutManager;
  final int mode;

  const QueryTrackListView({
    super.key,
    required this.layoutManager,
    required this.queries,
    required this.mode,
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
      final newItems = await queryMixTracks(
        widget.queries,
        cursor,
        _pageSize,
      );

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
    return TrackList(
      pagingController: _pagingController,
      queries: widget.queries,
      mode: widget.mode,
    );
  }

  @override
  void dispose() {
    _pagingController.dispose();
    super.dispose();
  }
}
