import '../../utils/api/request_failure.dart';
import '../../utils/l10n.dart';
import '../../utils/logger.dart';
import '../../utils/dialogs/information/error.dart';
import 'dart:async';
import 'dart:math';

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
import '../../widgets/start_screen/stacked_layout.dart';
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

  /// Fires when the library's content changes. Tests pass their own stream;
  /// it defaults to [LibraryContentChanged.rustSignalStream].
  final Stream<void>? contentChanged;

  /// Replaces how pages of a non-playlist query are loaded. Tests use it to
  /// drive the list without the native hub.
  final QueryTrackPageLoader loadMixPage;

  /// Replaces how pages of a playlist are loaded. Tests use it to drive the
  /// list without the native hub.
  final PlaylistPageLoader loadPlaylistPage;

  /// The layout to draw the tracks in instead of the one the window size
  /// picks, for a page that decides on its own.
  final DeviceType? layout;

  /// Set when the list is the child of a [StartGroupStackedLayout], which
  /// already pads its child. The large list then adds no padding above or to
  /// the left of its first column.
  final bool underTitle;

  /// Drawn as the first item of the small and the band lists, scrolling away
  /// with the tracks. It stays while the tracks load. The large list has no
  /// header.
  final Widget? header;

  const QueryTrackListView({
    super.key,
    required this.layoutManager,
    required this.queries,
    required this.mode,
    this.contentChanged,
    this.loadMixPage = queryMixTracks,
    this.loadPlaylistPage = fetchPlaylistOccurrences,
    this.layout,
    this.underTitle = false,
    this.header,
  });

  @override
  QueryTrackListViewState createState() => QueryTrackListViewState();
}

class QueryTrackListViewState extends State<QueryTrackListView> {
  static const _pageSize = 20;

  // How many tracks beyond one the list builds are loaded ahead of it, so a
  // track has arrived before its place scrolls into view.
  static const _loadAhead = 3 * _pageSize;

  int _totalCount = 0;
  final Map<int, InternalMediaFile> _loadedItems = {};
  final Set<int> _loadingIndices = {};
  // Later pages that failed. Rebuilds do not request them again, or a failure
  // that comes back at once would loop. A content change or new queries clear
  // them.
  final Set<int> _failedPages = {};
  final Map<int, PlaylistOccurrenceCursor?> _playlistCursors = {0: null};
  bool _isInitialized = false;
  bool _reloading = false;
  // Content changes that come while a reload runs make one more reload after
  // it, not one each.
  bool _reloadRunning = false;
  bool _reloadPending = false;
  // Whether this list's failure dialog is open, so that refreshes failing
  // while it is open add no second one.
  bool _showingFailure = false;
  // A failure the hub reported, or a request that got no answer in time.
  Object? _failure;
  bool _reachedEnd = false;
  int _generation = 0;
  StreamSubscription? _contentSubscription;

  @override
  void initState() {
    super.initState();
    final changes =
        widget.contentChanged ??
        LibraryContentChanged.rustSignalStream.map<void>((_) {});
    _contentSubscription = changes.listen((_) {
      // Failed pages may load now that the library changed.
      _failedPages.clear();
      _requestReload();
    });
    _initializeData();
  }

  // Starts the listing over from an empty page, for the page's own refresh.
  void _restartListing() {
    _generation++;
    _reloading = false;
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
    _reloading = false;
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

  // Playlist pages follow the cursors in [cursors], the list's own unless a
  // reload builds a new chain.
  Future<List<InternalMediaFile>> _fetchPage(
    int cursor, [
    Map<int, PlaylistOccurrenceCursor?>? cursors,
  ]) async {
    final playlistId = getPlaylistIdFromQueryList(widget.queries);
    if (playlistId == null) {
      return widget.loadMixPage(widget.queries, cursor, _pageSize);
    }
    final chain = cursors ?? _playlistCursors;
    if (!chain.containsKey(cursor)) {
      throw StateError('The preceding playlist page has not loaded');
    }
    final generation = _generation;
    final page = await widget.loadPlaylistPage(
      playlistId,
      chain[cursor],
      _pageSize,
    );
    if (mounted && generation == _generation && page.next != null) {
      chain[cursor + page.items.length] = page.next;
    }
    return page.items;
  }

  // Reloads now, or once more after the reload in flight, however many changes
  // arrive meanwhile.
  void _requestReload() {
    if (_reloadRunning) {
      _reloadPending = true;
      return;
    }
    unawaited(_runReload());
  }

  Future<void> _runReload() async {
    _reloadRunning = true;
    try {
      await _reloadInPlace();
    } finally {
      _reloadRunning = false;
    }
    if (_reloadPending && mounted) {
      _reloadPending = false;
      unawaited(_runReload());
    }
  }

  // Keeps the old list on screen until the fresh pages have all arrived, so a
  // scrolled list does not jump back to the start. If a page fails, the old
  // list and its cursors stay as they were.
  Future<void> _reloadInPlace() async {
    _generation++;
    final generation = _generation;
    _reloading = true;
    final furthest = _loadedItems.keys.fold<int>(-1, (a, b) => a > b ? a : b);
    // Playlist pages are chained by cursors, so they must be fetched in order,
    // into a new chain that replaces the old one once all have arrived.
    final cursors = <int, PlaylistOccurrenceCursor?>{0: null};
    final fresh = <int, InternalMediaFile>{};
    var reachedEnd = false;
    var totalCount = 0;
    try {
      var cursor = 0;
      while (true) {
        final newItems = await _fetchPage(cursor, cursors);
        if (!mounted || generation != _generation) return;
        for (var i = 0; i < newItems.length; i++) {
          fresh[cursor + i] = newItems[i];
        }
        final loadedCount = cursor + newItems.length;
        if (newItems.length < _pageSize ||
            (getPlaylistIdFromQueryList(widget.queries) != null &&
                !cursors.containsKey(loadedCount))) {
          reachedEnd = true;
          totalCount = loadedCount;
          break;
        }
        totalCount = loadedCount + _pageSize;
        cursor = loadedCount;
        if (cursor > furthest) break;
      }
    } catch (error) {
      if (!mounted || generation != _generation) return;
      setState(() {
        // The new generation dropped the pages that were loading.
        _loadingIndices.clear();
        _isInitialized = true;
        _reloading = false;
      });
      _loadFailed(error);
      return;
    }
    setState(() {
      _loadedItems
        ..clear()
        ..addAll(fresh);
      _playlistCursors
        ..clear()
        ..addAll(cursors);
      _loadingIndices.clear();
      _reachedEnd = reachedEnd;
      _totalCount = totalCount;
      _isInitialized = true;
      _reloading = false;
      _failure = null;
    });
    Timer(
      Duration(milliseconds: gridAnimationDelay),
      () => widget.layoutManager.playAnimations(),
    );
  }

  // Decides what a failed load shows. Tracks already on the page stay and the
  // failure shows in the error dialog, one at a time; with none, the page
  // shows the failure. Only the list of the page shown opens the dialog: the
  // lists of the pages stacked under it fail together with it, and the user
  // needs the error once.
  void _loadFailed(Object error) {
    if (_loadedItems.isEmpty) {
      if (_shownFailure(error)) setState(() => _failure = error);
      return;
    }
    if (_showingFailure || ModalRoute.isCurrentOf(context) == false) return;
    _showingFailure = true;
    final s = S.of(context);
    showErrorDialog(
      context: context,
      title: s.unknownError,
      errorMessage: requestErrorText(s, error),
    ).whenComplete(() => _showingFailure = false);
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
      _loadFailed(error);
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
      // Nothing moves on the page while it waits, so the frame that applies
      // what arrived is asked for.
      SchedulerBinding.instance.ensureVisualUpdate();

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
        SchedulerBinding.instance.ensureVisualUpdate();
        return;
      }

      SchedulerBinding.instance.addPostFrameCallback((_) {
        if (!mounted || generation != _generation) return;
        setState(() {
          _loadingIndices.remove(cursor);
          _failedPages.add(cursor);
        });
        if (!_shownFailure(error)) return;
        // Tracks already loaded stay on the page.
        if (_loadedItems.isNotEmpty) {
          logger.w('Failed to load tracks from $cursor: $error');
        }
        _loadFailed(error);
      });
      SchedulerBinding.instance.ensureVisualUpdate();
    }
  }

  // Failures the page shows instead of its tracks. A request that got no answer in time would
  // otherwise leave the page empty.
  static bool _shownFailure(Object error) =>
      error is RequestFailure || error is TimeoutException;

  void _checkAndLoadItem(int index) {
    if (_reloading) return;
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
    // Pages load one after another, so the one asked for ahead is at most the
    // first that has not loaded, the last the count reaches.
    final ahead = min(index + _loadAhead, _totalCount - 1);
    if (ahead > index) _checkAndLoadItem(ahead);
    return _loadedItems[index];
  }

  @override
  Widget build(BuildContext context) {
    if (_failure != null) {
      return Center(child: Text(requestErrorText(S.of(context), _failure!)));
    }
    if (!_isInitialized && widget.header == null) {
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
      builder: (context, detected) {
        final activeBreakpoint = widget.layout ?? detected;
        if (activeBreakpoint == DeviceType.belt) {
          return BeltContainer(
            child: BandScreenTrackList(
              totalCount: _totalCount,
              getItem: _getItem,
              queries: widget.queries,
              mode: widget.mode,
              reloadData: _restartListing,
              header: widget.header,
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
            reloadData: _restartListing,
            header: widget.header,
          );
        }

        if (activeBreakpoint == DeviceType.zune) {
          return SmallScreenTrackList(
            totalCount: _totalCount,
            getItem: _getItem,
            queries: widget.queries,
            mode: widget.mode,
            reloadData: _restartListing,
            header: widget.header,
          );
        }

        return LargeScreenTrackList(
          totalCount: _totalCount,
          getItem: _getItem,
          queries: widget.queries,
          mode: widget.mode,
          reloadData: _restartListing,
          underTitle: widget.underTitle,
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
