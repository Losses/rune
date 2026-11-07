import '../../utils/api/request_failure.dart';
import '../../utils/l10n.dart';
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

  int _totalCount = 0;
  final Map<int, InternalMediaFile> _loadedItems = {};
  final Set<int> _loadingIndices = {};
  final Map<int, PlaylistOccurrenceCursor?> _playlistCursors = {0: null};
  bool _isInitialized = false;
  RequestFailure? _failure;
  bool _reachedEnd = false;
  int _generation = 0;
  StreamSubscription? _contentSubscription;

  @override
  void initState() {
    super.initState();
    _contentSubscription = LibraryContentChanged.rustSignalStream.listen((_) {
      _generation++;
      _loadedItems.clear();
      _playlistCursors
        ..clear()
        ..[0] = null;
      _loadingIndices.clear();
      _reachedEnd = false;
      _totalCount = 0;
      setState(() {
        _isInitialized = false;
        _failure = null;
      });
      _initializeData();
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
      return queryMixTracks(widget.queries, cursor, _pageSize);
    }
    if (!_playlistCursors.containsKey(cursor)) {
      throw StateError('The preceding playlist page has not loaded');
    }
    final generation = _generation;
    final page = await fetchPlaylistOccurrences(
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
      if (error is RequestFailure) _failure = error;
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
