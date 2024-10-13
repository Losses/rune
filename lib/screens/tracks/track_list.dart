import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:infinite_scroll_pagination/infinite_scroll_pagination.dart';

import '../../utils/query_list.dart';
import '../../utils/api/fetch_media_files.dart';
import '../../config/animation.dart';
import '../../widgets/track_list/band_screen_track_list.dart';
import '../../widgets/track_list/small_screen_track_list.dart';
import '../../widgets/track_list/large_screen_track_list.dart';
import '../../widgets/track_list/utils/internal_media_file.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../providers/responsive_providers.dart';

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

  final PagingController<int, InternalMediaFile> _pagingController =
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
      final newItems = await fetchMediaFiles(cursor, _pageSize);

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
    const queries = QueryList([("lib::all", "true")]);
    return BreakpointBuilder(
      breakpoints: const [DeviceType.band, DeviceType.zune, DeviceType.tv],
      builder: (context, activeBreakpoint) {
        if (activeBreakpoint == DeviceType.band) {
          return BandScreenTrackList(
            pagingController: _pagingController,
            queries: queries,
            mode: 99,
          );
        }

        if (activeBreakpoint == DeviceType.zune) {
          return SmallScreenTrackList(
            pagingController: _pagingController,
            queries: queries,
            mode: 99,
          );
        }

        return LargeScreenTrackList(
          pagingController: _pagingController,
          queries: queries,
          mode: 99,
        );
      },
    );
  }

  @override
  void dispose() {
    _pagingController.dispose();
    super.dispose();
  }
}
