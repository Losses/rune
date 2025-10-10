import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/scheduler.dart';

import '../../utils/query_list.dart';
import '../../utils/api/fetch_media_files.dart';
import '../../utils/api/get_media_files_count.dart';
import '../../config/animation.dart';
import '../../widgets/belt_container.dart';
import '../../widgets/track_list/band_screen_track_list.dart';
import '../../widgets/track_list/small_screen_track_list.dart';
import '../../widgets/track_list/large_screen_track_list.dart';
import '../../widgets/track_list/utils/internal_media_file.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../providers/responsive_providers.dart';

class TrackListView extends StatefulWidget {
  final StartScreenLayoutManager layoutManager;

  const TrackListView({super.key, required this.layoutManager});

  @override
  TrackListViewState createState() => TrackListViewState();
}

class TrackListViewState extends State<TrackListView> {
  static const _pageSize = 100;

  int _totalCount = 0;
  final Map<int, InternalMediaFile> _loadedItems = {};
  final Set<int> _loadingIndices = {};
  bool _isInitialized = false;

  @override
  void initState() {
    super.initState();
    _initializeData();
  }

  Future<void> _initializeData() async {
    try {
      final count = await getMediaFilesCount();
      if (!mounted) return;

      setState(() {
        _totalCount = count;
        _isInitialized = true;
      });
      // Pre-load first page
      _loadPage(0);
    } catch (error) {
      // If getting count fails, fall back to dynamic loading
      if (!mounted) return;

      setState(() {
        _totalCount = 10000; // Set a large number as fallback
        _isInitialized = true;
      });
      _loadPage(0);
    }
  }

  Future<void> _loadPage(int cursor) async {
    if (_loadingIndices.contains(cursor)) return;

    // Immediately mark as loading to prevent duplicate requests
    _loadingIndices.add(cursor);

    // Schedule setState for after the current build phase to update UI
    SchedulerBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      setState(() {});
    });

    try {
      final newItems = await fetchMediaFiles(cursor, _pageSize);
      if (!mounted) return;

      SchedulerBinding.instance.addPostFrameCallback((_) {
        if (!mounted) return;
        setState(() {
          for (var i = 0; i < newItems.length; i++) {
            _loadedItems[cursor + i] = newItems[i];
          }
          _loadingIndices.remove(cursor);
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
    if (_loadedItems.containsKey(index)) return;
    if (_loadingIndices.contains((index ~/ _pageSize) * _pageSize)) return;

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

    const queries = QueryList([("lib::all", "true")]);
    return DeviceTypeBuilder(
      deviceType: const [
        DeviceType.dock,
        DeviceType.zune,
        DeviceType.tv,
        DeviceType.band,
        DeviceType.belt,
      ],
      builder: (context, activeBreakpoint) {
        if (activeBreakpoint == DeviceType.belt) {
          return BeltContainer(
            child: BandScreenTrackList(
              totalCount: _totalCount,
              getItem: _getItem,
              queries: queries,
              mode: 99,
              topPadding: true,
            ),
          );
        }

        if (activeBreakpoint == DeviceType.dock ||
            activeBreakpoint == DeviceType.band) {
          return BandScreenTrackList(
            totalCount: _totalCount,
            getItem: _getItem,
            queries: queries,
            mode: 99,
            topPadding: true,
          );
        }

        if (activeBreakpoint == DeviceType.zune) {
          return SmallScreenTrackList(
            totalCount: _totalCount,
            getItem: _getItem,
            queries: queries,
            mode: 99,
            topPadding: true,
          );
        }

        return LargeScreenTrackList(
          totalCount: _totalCount,
          getItem: _getItem,
          queries: queries,
          mode: 99,
          topPadding: true,
        );
      },
    );
  }

  @override
  void dispose() {
    super.dispose();
  }
}
