import 'dart:async';
import 'dart:math';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/scheduler.dart';

import '../../bindings/bindings.dart';
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

  /// Counts the tracks of the library. Tests pass a fake.
  final Future<int> Function() loadCount;

  /// Loads a page of the tracks from a cursor. Tests pass a fake.
  final Future<List<InternalMediaFile>> Function(int cursor, int pageSize)
  loadPage;

  const TrackListView({
    super.key,
    required this.layoutManager,
    this.loadCount = getMediaFilesCount,
    this.loadPage = fetchMediaFiles,
  });

  @override
  TrackListViewState createState() => TrackListViewState();
}

class TrackListViewState extends State<TrackListView> {
  static const _pageSize = 100;

  int _totalCount = 0;
  final Map<int, InternalMediaFile> _loadedItems = {};
  // Shown only until the fresh page arrives, so a reload keeps the list on screen.
  final Map<int, InternalMediaFile> _staleItems = {};
  final Set<int> _loadingIndices = {};
  // Pages whose request failed. Building their places asks for them again, so
  // a page that keeps failing would be requested on every rebuild. They are
  // skipped when loading ahead, and asked for once more when their own places
  // are built. A content change or a restart clears them.
  final Set<int> _failedPages = {};
  final Set<int> _retriedPages = {};
  bool _isInitialized = false;
  // Whether the first page has come, empty or not, or has failed.
  bool _firstPageSettled = false;
  int _generation = 0;
  StreamSubscription? _contentSubscription;

  @override
  void initState() {
    super.initState();
    _contentSubscription = LibraryContentChanged.rustSignalStream.listen(
      (_) => _restartListing(),
    );
    _initializeData();
  }

  void _restartListing() {
    // The list is built again, so that the ring shows while the count of the
    // restart is pending.
    setState(() {
      _generation++;
      _staleItems.addAll(_loadedItems);
      _loadedItems.clear();
      _loadingIndices.clear();
      _failedPages.clear();
      _retriedPages.clear();
      _firstPageSettled = false;
    });
    _initializeData();
  }

  Future<void> _initializeData() async {
    final generation = _generation;
    try {
      final count = await widget.loadCount();
      if (!mounted || generation != _generation) return;

      setState(() {
        _totalCount = count;
        _isInitialized = true;
        // An empty list has no first page to wait for.
        if (count == 0) _firstPageSettled = true;
        _staleItems.removeWhere((index, _) => index >= count);
      });
      // Pre-load first page
      _loadPage(0);
    } catch (error) {
      // If getting count fails, fall back to dynamic loading
      if (!mounted || generation != _generation) return;

      setState(() {
        _totalCount = 10000; // Set a large number as fallback
        _isInitialized = true;
      });
      _loadPage(0);
    }
  }

  Future<void> _loadPage(int cursor) async {
    final generation = _generation;
    if (_loadingIndices.contains(cursor)) return;

    // Immediately mark as loading to prevent duplicate requests
    _loadingIndices.add(cursor);

    // Schedule setState for after the current build phase to update UI
    SchedulerBinding.instance.addPostFrameCallback((_) {
      if (!mounted || generation != _generation) return;
      setState(() {});
    });

    try {
      final newItems = await widget.loadPage(cursor, _pageSize);
      if (!mounted || generation != _generation) return;

      SchedulerBinding.instance.addPostFrameCallback((_) {
        if (!mounted || generation != _generation) return;
        setState(() {
          for (var i = 0; i < _pageSize; i++) {
            _staleItems.remove(cursor + i);
          }
          for (var i = 0; i < newItems.length; i++) {
            _loadedItems[cursor + i] = newItems[i];
          }
          _loadingIndices.remove(cursor);
          // A short page is the end of the list, which stops the pages ahead
          // of it from being asked for when the count was not known.
          if (newItems.length < _pageSize) {
            _totalCount = min(_totalCount, cursor + newItems.length);
          }
          if (cursor == 0) _firstPageSettled = true;
        });
      });
      // Nothing moves on the page while it waits, so the frame that applies
      // what arrived is asked for.
      SchedulerBinding.instance.ensureVisualUpdate();

      Timer(
        Duration(milliseconds: gridAnimationDelay),
        () => widget.layoutManager.playAnimations(),
      );
    } catch (error) {
      if (!mounted || generation != _generation) return;

      SchedulerBinding.instance.addPostFrameCallback((_) {
        if (!mounted || generation != _generation) return;
        setState(() {
          _loadingIndices.remove(cursor);
          _failedPages.add(cursor);
          if (cursor == 0) _firstPageSettled = true;
        });
      });
      SchedulerBinding.instance.ensureVisualUpdate();
    }
  }

  void _checkAndLoadItem(int index, {bool ahead = false}) {
    if (_loadedItems.containsKey(index)) return;

    final pageStart = (index ~/ _pageSize) * _pageSize;
    if (_loadingIndices.contains(pageStart)) return;
    if (_failedPages.contains(pageStart)) {
      if (ahead || !_retriedPages.add(pageStart)) return;
      _failedPages.remove(pageStart);
    }
    _loadPage(pageStart);
  }

  InternalMediaFile? _getItem(int index) {
    _checkAndLoadItem(index);
    // The page after the one the list builds is loaded ahead of it, so a track
    // has arrived before its place scrolls into view.
    final ahead = min(index + _pageSize, _totalCount - 1);
    if (ahead > index) _checkAndLoadItem(ahead, ahead: true);
    return _loadedItems[index] ?? _staleItems[index];
  }

  @override
  Widget build(BuildContext context) {
    // The ring stays until the first page has come or failed, so the list is
    // never a screen of blank places. A reload keeps the stale tracks instead.
    final waitingForFirstPage = !_firstPageSettled && _staleItems.isEmpty;
    if (!_isInitialized || waitingForFirstPage) {
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
              mode: null,
              reloadData: _restartListing,
            ),
          );
        }

        if (activeBreakpoint == DeviceType.dock ||
            activeBreakpoint == DeviceType.band) {
          return BandScreenTrackList(
            totalCount: _totalCount,
            getItem: _getItem,
            queries: queries,
            mode: null,
            reloadData: _restartListing,
          );
        }

        if (activeBreakpoint == DeviceType.zune) {
          return SmallScreenTrackList(
            totalCount: _totalCount,
            getItem: _getItem,
            queries: queries,
            mode: null,
            reloadData: _restartListing,
          );
        }

        return LargeScreenTrackList(
          totalCount: _totalCount,
          getItem: _getItem,
          queries: queries,
          mode: null,
          reloadData: _restartListing,
        );
      },
    );
  }

  @override
  void dispose() {
    _contentSubscription?.cancel();
    super.dispose();
  }
}
