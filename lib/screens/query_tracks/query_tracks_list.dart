import '../../utils/api/request_failure.dart';
import '../../utils/l10n.dart';
import '../../utils/logger.dart';
import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/scheduler.dart';

import '../../bindings/bindings.dart';
import '../../utils/query_list.dart';
import '../../utils/api/query_mix_tracks.dart';
import '../../utils/api/fetch_playlist_occurrences.dart';
import '../../utils/get_playlist_id_from_query_list.dart';
import '../../config/animation.dart';
import '../../widgets/belt_container.dart';
import '../../widgets/track_list/band_screen_track_list.dart';
import '../../widgets/track_list/large_screen_track_list.dart';
import '../../widgets/track_list/small_screen_track_list.dart';
import '../../widgets/track_list/utils/internal_media_file.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../providers/responsive_providers.dart';

/// Loads one page of a mix query. The production loader is [queryMixTracks].
typedef QueryTrackPageLoader = Future<List<InternalMediaFile>> Function(
  QueryList queries,
  int cursor,
  int pageSize,
);

/// Loads one page of a playlist's entries. The production loader is
/// [fetchPlaylistOccurrences].
typedef PlaylistPageLoader =
    Future<PlaylistOccurrencePage> Function(
      int playlistId,
      PlaylistOccurrenceCursor? after,
      int pageSize,
    );

class QueryTrackListView extends StatefulWidget {
  final QueryList queries;
  final StartScreenLayoutManager layoutManager;
  final PlaybackMode? mode;

  /// Replaces how pages of a non-playlist query are loaded. Tests use it to
  /// drive the list without the native hub.
  final QueryTrackPageLoader loadMixPage;

  /// Replaces how pages of a playlist are loaded. Tests use it to drive the
  /// list without the native hub.
  final PlaylistPageLoader loadPlaylistPage;

  const QueryTrackListView({
    super.key,
    required this.layoutManager,
    required this.queries,
    required this.mode,
    this.loadMixPage = queryMixTracks,
    this.loadPlaylistPage = fetchPlaylistOccurrences,
  });

  @override
  QueryTrackListViewState createState() => QueryTrackListViewState();
}

class QueryTrackListViewState extends State<QueryTrackListView> {
  static const _pageSize = 20;

  int _totalCount = 0;
  final Map<int, InternalMediaFile> _loadedItems = {};
  final Set<int> _loadingIndices = {};
  // Later pages that failed. Rebuilds do not request them again, or a failure
  // that comes back at once would loop. A content change or new queries clear
  // them.
  final Set<int> _failedPages = {};
  final Map<int, PlaylistOccurrenceCursor?> _playlistCursors = {0: null};
  bool _isInitialized = false;
  // A failure the hub reported, or a request that got no answer in time.
  Object? _failure;
  bool _reachedEnd = false;
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
    _generation++;
    _loadedItems.clear();
    _playlistCursors
      ..clear()
      ..[0] = null;
    _loadingIndices.clear();
    _failedPages.clear();
    _reachedEnd = false;
    _totalCount = 0;
    setState(() {
      _isInitialized = false;
      _failure = null;
    });
    _initializeData();
  }

  @override
  void didUpdateWidget(covariant QueryTrackListView oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.queries == widget.queries) return;
    _generation++;
    _loadedItems.clear();
    _loadingIndices.clear();
    _failedPages.clear();
    _playlistCursors
      ..clear()
      ..[0] = null;
    _reachedEnd = false;
    _totalCount = 0;
    _isInitialized = false;
    _failure = null;
    _initializeData();
  }

  Future<List<InternalMediaFile>> _fetchPage(int cursor) async {
    final playlistId = getPlaylistIdFromQueryList(widget.queries);
    if (playlistId == null) {
      return widget.loadMixPage(widget.queries, cursor, _pageSize);
    }
    if (!_playlistCursors.containsKey(cursor)) {
      throw StateError('The preceding playlist page has not loaded');
    }
    final generation = _generation;
    final page = await widget.loadPlaylistPage(
      playlistId,
      _playlistCursors[cursor],
      _pageSize,
    );
    if (mounted && generation == _generation && page.next != null) {
      _playlistCursors[cursor + page.items.length] = page.next;
    }
    return page.items;
  }

  Future<void> _initializeData() async {
    final generation = _generation;
    // Pre-load first page before marking as initialized
    await _loadPageForInitialization(0);
    if (!mounted || generation != _generation) return;
    setState(() {
      _isInitialized = true;
    });
  }

  Future<void> _loadPageForInitialization(int cursor) async {
    final generation = _generation;
    if (_loadingIndices.contains(cursor) || _reachedEnd) return;

    _loadingIndices.add(cursor);

    try {
      final newItems = await _fetchPage(cursor);

      if (!mounted || generation != _generation) return;

      for (var i = 0; i < newItems.length; i++) {
        _loadedItems[cursor + i] = newItems[i];
      }
      _loadingIndices.remove(cursor);

      // Check if we've reached the end
      if (newItems.length < _pageSize ||
          (getPlaylistIdFromQueryList(widget.queries) != null &&
              !_playlistCursors.containsKey(cursor + newItems.length))) {
        _reachedEnd = true;
        _totalCount = cursor + newItems.length;
      } else {
        final loadedCount = cursor + newItems.length;
        _totalCount = loadedCount + _pageSize;
      }

      Timer(
        Duration(milliseconds: gridAnimationDelay),
        () => widget.layoutManager.playAnimations(),
      );
    } catch (error) {
      if (!mounted || generation != _generation) return;
      _loadingIndices.remove(cursor);
      if (_shownFailure(error)) _failure = error;
    }
  }

  Future<void> _loadPage(int cursor) async {
    final generation = _generation;
    if (_loadingIndices.contains(cursor) || _reachedEnd) return;

    // Immediately mark as loading to prevent duplicate requests
    _loadingIndices.add(cursor);

    // Schedule setState for after the current build phase to update UI
    SchedulerBinding.instance.addPostFrameCallback((_) {
      if (!mounted || generation != _generation) return;
      setState(() {});
    });

    try {
      final newItems = await _fetchPage(cursor);

      if (!mounted || generation != _generation) return;

      SchedulerBinding.instance.addPostFrameCallback((_) {
        if (!mounted || generation != _generation) return;
        setState(() {
          for (var i = 0; i < newItems.length; i++) {
            _loadedItems[cursor + i] = newItems[i];
          }
          _loadingIndices.remove(cursor);

          // Check if we've reached the end
          if (newItems.length < _pageSize ||
              (getPlaylistIdFromQueryList(widget.queries) != null &&
                  !_playlistCursors.containsKey(cursor + newItems.length))) {
            _reachedEnd = true;
            _totalCount = cursor + newItems.length;
          } else {
            // If we haven't reached the end, assume there's at least one more page
            // This allows the UI to scroll and trigger loading the next page
            final loadedCount = cursor + newItems.length;
            _totalCount = loadedCount + _pageSize;
          }
        });
      });

      Timer(
        Duration(milliseconds: gridAnimationDelay),
        () => widget.layoutManager.playAnimations(),
      );
    } catch (error) {
      if (!mounted || generation != _generation) return;

      // A later playlist page conflicts when the playlist changed after its first
      // page was listed, as when another client edits a Subsonic playlist. The
      // cursors no longer fit, so list the playlist again from its first page,
      // which cannot conflict.
      if (error is RequestFailure &&
          error.kind == RequestErrorKind.conflict &&
          getPlaylistIdFromQueryList(widget.queries) != null) {
        SchedulerBinding.instance.addPostFrameCallback((_) {
          if (!mounted || generation != _generation) return;
          _restartListing();
        });
        return;
      }

      SchedulerBinding.instance.addPostFrameCallback((_) {
        if (!mounted || generation != _generation) return;
        setState(() {
          _loadingIndices.remove(cursor);
          _failedPages.add(cursor);
          if (!_shownFailure(error)) return;
          // Tracks already loaded stay on the page.
          if (_loadedItems.isEmpty) {
            _failure = error;
          } else {
            logger.w('Failed to load tracks from $cursor: $error');
          }
        });
      });
    }
  }

  // Failures the page shows instead of its tracks. A request that got no answer in time would
  // otherwise leave the page empty.
  static bool _shownFailure(Object error) =>
      error is RequestFailure || error is TimeoutException;

  void _checkAndLoadItem(int index) {
    if (_loadedItems.containsKey(index) ||
        (index >= _totalCount && _reachedEnd)) {
      return;
    }
    final pageStart = (index ~/ _pageSize) * _pageSize;
    if (_loadingIndices.contains(pageStart) ||
        _failedPages.contains(pageStart)) {
      return;
    }

    _loadPage(pageStart);
  }

  InternalMediaFile? _getItem(int index) {
    _checkAndLoadItem(index);
    return _loadedItems[index];
  }

  @override
  Widget build(BuildContext context) {
    if (_failure != null) {
      return Center(child: Text(requestErrorText(S.of(context), _failure!)));
    }
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
          );
        }

        if (activeBreakpoint == DeviceType.zune) {
          return SmallScreenTrackList(
            totalCount: _totalCount,
            getItem: _getItem,
            queries: widget.queries,
            mode: widget.mode,
          );
        }

        return LargeScreenTrackList(
          totalCount: _totalCount,
          getItem: _getItem,
          queries: widget.queries,
          mode: widget.mode,
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
