import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/scheduler.dart';

import '../../utils/query_list.dart';
import '../../utils/api/query_mix_tracks.dart';
import '../../config/animation.dart';
import '../../widgets/belt_container.dart';
import '../../widgets/track_list/band_screen_track_list.dart';
import '../../widgets/track_list/large_screen_track_list.dart';
import '../../widgets/track_list/small_screen_track_list.dart';
import '../../widgets/track_list/utils/internal_media_file.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../providers/responsive_providers.dart';

class QueryTrackListView extends StatefulWidget {
  final QueryList queries;
  final StartScreenLayoutManager layoutManager;
  final int mode;
  final bool topPadding;

  const QueryTrackListView({
    super.key,
    required this.layoutManager,
    required this.queries,
    required this.mode,
    required this.topPadding,
  });

  @override
  QueryTrackListViewState createState() => QueryTrackListViewState();
}

class QueryTrackListViewState extends State<QueryTrackListView> {
  static const _pageSize = 20;

  int _totalCount = 0;
  final Map<int, InternalMediaFile> _loadedItems = {};
  final Set<int> _loadingIndices = {};
  bool _isInitialized = false;
  bool _reachedEnd = false;

  @override
  void initState() {
    super.initState();
    _initializeData();
  }

  Future<void> _initializeData() async {
    setState(() {
      _isInitialized = true;
    });
    // Pre-load first page
    _loadPage(0);
  }

  Future<void> _loadPage(int cursor) async {
    if (_loadingIndices.contains(cursor) || _reachedEnd) return;

    // Immediately mark as loading to prevent duplicate requests
    _loadingIndices.add(cursor);

    // Schedule setState for after the current build phase to update UI
    SchedulerBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      setState(() {});
    });

    try {
      final newItems = await queryMixTracks(widget.queries, cursor, _pageSize);

      if (!mounted) return;

      SchedulerBinding.instance.addPostFrameCallback((_) {
        if (!mounted) return;
        setState(() {
          for (var i = 0; i < newItems.length; i++) {
            _loadedItems[cursor + i] = newItems[i];
          }
          _loadingIndices.remove(cursor);

          // Update total count
          final newTotal = cursor + newItems.length;
          if (newTotal > _totalCount) {
            _totalCount = newTotal;
          }

          // Check if we've reached the end
          if (newItems.length < _pageSize) {
            _reachedEnd = true;
            _totalCount = cursor + newItems.length;
          }
        });
      });

      Timer(
        Duration(milliseconds: gridAnimationDelay),
        () => widget.layoutManager.playAnimations(),
      );
    } catch (error) {
      if (!mounted) return;

      SchedulerBinding.instance.addPostFrameCallback((_) {
        if (!mounted) return;
        setState(() {
          _loadingIndices.remove(cursor);
        });
      });
    }
  }

  void _checkAndLoadItem(int index) {
    if (_loadedItems.containsKey(index) ||
        (index >= _totalCount && _reachedEnd)) {
      return;
    }
    if (_loadingIndices.contains((index ~/ _pageSize) * _pageSize)) {
      return;
    }

    final pageStart = (index ~/ _pageSize) * _pageSize;
    _loadPage(pageStart);
  }

  InternalMediaFile? _getItem(int index) {
    _checkAndLoadItem(index);
    return _loadedItems[index];
  }

  @override
  Widget build(BuildContext context) {
    if (!_isInitialized) {
      return const Center(child: ProgressRing());
    }

    return DeviceTypeBuilder(
      deviceType: const [
        DeviceType.band,
        DeviceType.belt,
        DeviceType.dock,
        DeviceType.zune,
        DeviceType.tv,
      ],
      builder: (context, activeBreakpoint) {
        if (activeBreakpoint == DeviceType.belt) {
          return BeltContainer(
            child: BandScreenTrackList(
              totalCount: _totalCount,
              getItem: _getItem,
              queries: widget.queries,
              mode: widget.mode,
              topPadding: widget.topPadding,
            ),
          );
        }

        if (activeBreakpoint == DeviceType.dock ||
            activeBreakpoint == DeviceType.band) {
          return BandScreenTrackList(
            totalCount: _totalCount,
            getItem: _getItem,
            queries: widget.queries,
            mode: widget.mode,
            topPadding: widget.topPadding,
          );
        }

        if (activeBreakpoint == DeviceType.zune) {
          return SmallScreenTrackList(
            totalCount: _totalCount,
            getItem: _getItem,
            queries: widget.queries,
            mode: widget.mode,
            topPadding: widget.topPadding,
          );
        }

        return LargeScreenTrackList(
          totalCount: _totalCount,
          getItem: _getItem,
          queries: widget.queries,
          mode: widget.mode,
          topPadding: widget.topPadding,
        );
      },
    );
  }

  @override
  void dispose() {
    super.dispose();
  }
}
