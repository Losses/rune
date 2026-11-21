import '../../utils/api/request_failure.dart';
import '../../utils/l10n.dart';
import '../../utils/dialogs/information/error.dart';
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

class QueryTrackListView extends StatefulWidget {
  final QueryList queries;
  final StartScreenLayoutManager layoutManager;
  final PlaybackMode? mode;

  /// Fires when the library's content changes. Tests pass their own stream;
  /// it defaults to [LibraryContentChanged.rustSignalStream].
  final Stream<void>? contentChanged;

  /// Loads a page of the tracks of [queries] that are not a playlist, from a
  /// cursor and with a page size. Tests pass a fake; it defaults to
  /// [queryMixTracks].
  final Future<List<InternalMediaFile>> Function(QueryList, int, int)?
  loadMixPage;

  /// Loads a page of a playlist's tracks after a cursor. Tests pass a fake;
  /// it defaults to [fetchPlaylistOccurrences].
  final Future<PlaylistOccurrencePage> Function(
    int,
    PlaylistOccurrenceCursor?,
    int,
  )?
  loadPlaylistPage;

  const QueryTrackListView({
    super.key,
    required this.layoutManager,
    required this.queries,
    required this.mode,
    this.contentChanged,
    this.loadMixPage,
    this.loadPlaylistPage,
  });

  @override
  QueryTrackListViewState createState() => QueryTrackListViewState();
}

class QueryTrackListViewState extends State<QueryTrackListView> {
  static const _pageSize = 20;

  int _totalCount = 0;
  final Map<int, InternalMediaFile> _loadedItems = {};
  final Set<int> _loadingIndices = {};
  final Map<int, PlaylistOccurrenceCursor?> _playlistCursors = {0: null};
  bool _isInitialized = false;
  bool _reloading = false;
  bool _showingFailure = false;
  RequestFailure? _failure;
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
      unawaited(_reloadInPlace());
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
      final loadMixPage = widget.loadMixPage ?? queryMixTracks;
      return loadMixPage(widget.queries, cursor, _pageSize);
    }
    final chain = cursors ?? _playlistCursors;
    if (!chain.containsKey(cursor)) {
      throw StateError('The preceding playlist page has not loaded');
    }
    final generation = _generation;
    final loadPlaylistPage =
        widget.loadPlaylistPage ?? fetchPlaylistOccurrences;
    final page = await loadPlaylistPage(playlistId, chain[cursor], _pageSize);
    if (mounted && generation == _generation && page.next != null) {
      chain[cursor + page.items.length] = page.next;
    }
    return page.items;
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
  // shows the failure.
  void _loadFailed(Object error) {
    if (_loadedItems.isEmpty) {
      if (error is RequestFailure) setState(() => _failure = error);
      return;
    }
    if (_showingFailure) return;
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
          if (error is RequestFailure) _failure = error;
        });
      });
    }
  }

  void _checkAndLoadItem(int index) {
    if (_reloading) return;
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
    if (_failure != null) {
      return Center(
        child: Text(requestFailureMessage(S.of(context), _failure!)),
      );
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
