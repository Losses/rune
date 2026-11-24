import 'dart:async';
import 'dart:math';

import 'package:flutter/foundation.dart';

import '../../../widgets/start_screen/utils/group.dart';
import '../../../widgets/start_screen/utils/internal_collection.dart';
import '../../../bindings/bindings.dart';

import 'fetch_page.dart';
import 'fetch_groups.dart';
import 'fetch_summary.dart';

class CollectionCache {
  static final CollectionCache _instance = CollectionCache._internal();

  factory CollectionCache() {
    return _instance;
  }

  CollectionCache._internal();

  int _generation = 0;
  int get generation => _generation;

  final _summaryCache = <CollectionType, List<Group<InternalCollection>>>{};
  final _pageCache =
      <CollectionType, Map<int, List<Group<InternalCollection>>>>{};
  final _groupCache =
      <CollectionType, Map<String, List<Group<InternalCollection>>>>{};

  void cacheSummary(CollectionType type, List<Group<InternalCollection>> data) {
    _summaryCache[type] = data;
  }

  void cachePage(
    CollectionType type,
    int cursor,
    List<Group<InternalCollection>> data,
  ) {
    if (!_pageCache.containsKey(type)) {
      _pageCache[type] = {};
    }
    _pageCache[type]![cursor] = data;
  }

  void cacheGroups(
    CollectionType type,
    String groupKey,
    List<Group<InternalCollection>> data,
  ) {
    if (!_groupCache.containsKey(type)) {
      _groupCache[type] = {};
    }
    _groupCache[type]![groupKey] = data;
  }

  List<Group<InternalCollection>>? getSummary(CollectionType type) {
    return _summaryCache[type];
  }

  List<Group<InternalCollection>>? getPage(CollectionType type, int cursor) {
    return _pageCache[type]?[cursor];
  }

  List<Group<InternalCollection>>? getGroups(
    CollectionType type,
    String groupKey,
  ) {
    return _groupCache[type]?[groupKey];
  }

  void clearType(CollectionType type) {
    _generation++;
    _summaryCache.remove(type);
    _pageCache.remove(type);
    _groupCache.remove(type);
  }

  void clearAll() {
    _generation++;
    _summaryCache.clear();
    _pageCache.clear();
    _groupCache.clear();
  }
}

class CollectionDataProvider with ChangeNotifier {
  static const _pageSize = 3;
  final CollectionType collectionType;
  final _cache = CollectionCache();

  late final StreamSubscription _contentSubscription;
  int _generation = 0;
  bool _disposed = false;

  final Future<List<Group<InternalCollection>>> Function(CollectionType)
  _loadSummary;
  final Future<(List<Group<InternalCollection>>, bool)> Function(
    CollectionType,
    int,
    int,
  )
  _pageLoader;

  // The in-flight reload, if any.
  Future<void>? _reloading;
  // A page asked for while a reload ran or was due. The reload fetches it
  // after the pages on screen.
  bool _pageRequested = false;
  // Set when a reload failed. Its old items stay on screen, but the next
  // request reloads from page zero: appending at their cursor would mix old
  // and new pages.
  bool _needsFullReload = false;

  /// Creates the provider of [collectionType]'s pages.
  ///
  /// The other parameters let tests stand in for the hub and default to it:
  /// - [contentChanged] fires when the library's content changes, in place of
  ///   [LibraryContentChanged.rustSignalStream];
  /// - [loadSummary] loads a type's summary, in place of
  ///   [fetchCollectionPageSummary];
  /// - [loadPage] loads the page of a type at a page size and cursor, and
  ///   whether it is the last, in place of [fetchCollectionPagePage].
  CollectionDataProvider({
    required this.collectionType,
    Stream<void>? contentChanged,
    Future<List<Group<InternalCollection>>> Function(CollectionType)?
    loadSummary,
    Future<(List<Group<InternalCollection>>, bool)> Function(
      CollectionType,
      int,
      int,
    )?
    loadPage,
  }) : _loadSummary = loadSummary ?? fetchCollectionPageSummary,
       _pageLoader = loadPage ?? fetchCollectionPagePage {
    final changes =
        contentChanged ??
        LibraryContentChanged.rustSignalStream.map<void>((_) {});
    _contentSubscription = changes.listen((_) {
      unawaited(reloadData());
    });
  }

  @override
  void dispose() {
    _disposed = true;
    _generation++;
    _pageRequest = null;
    _contentSubscription.cancel();
    super.dispose();
  }

  late Future<List<Group<InternalCollection>>> summary =
      _fetchSummaryWithCache();

  List<Group<InternalCollection>> items = [];

  Future<void>? _pageRequest;
  bool get isLoading => _pageRequest != null;

  /// The failure for the current page. Starting a retry clears it.
  Object? pageError;
  bool isLastPage = false;
  bool initialized = false;
  int cursor = 0;

  Future<List<Group<InternalCollection>>> _fetchSummaryWithCache() async {
    final cachedSummary = _cache.getSummary(collectionType);
    if (cachedSummary != null) {
      return cachedSummary;
    }

    final generation = _generation;
    final cacheGeneration = _cache.generation;
    final data = await _loadSummary(collectionType);
    if (generation == _generation &&
        cacheGeneration == _cache.generation &&
        !_disposed) {
      _cache.cacheSummary(collectionType, data);
    }
    return data;
  }

  Future<(List<Group<InternalCollection>>, bool)> _fetchPage(int cursor) async {
    final cachedPage = _cache.getPage(collectionType, cursor);
    if (cachedPage != null) {
      return (cachedPage, cachedPage.length < _pageSize);
    }

    final generation = _generation;
    final cacheGeneration = _cache.generation;
    final (newItems, isLastPage) = await _pageLoader(
      collectionType,
      _pageSize,
      cursor,
    );
    if (generation == _generation &&
        cacheGeneration == _cache.generation &&
        !_disposed) {
      _cache.cachePage(collectionType, cursor, newItems);
    }
    return (newItems, isLastPage);
  }

  Future<List<Group<InternalCollection>>> fetchGroups(
    List<String> groupTitles,
  ) async {
    // For simplicity, use concatenated groupTitles as the cache key
    final groupKey = groupTitles.join(',');
    final cachedGroups = _cache.getGroups(collectionType, groupKey);
    if (cachedGroups != null) {
      return cachedGroups;
    }

    final generation = _generation;
    final cacheGeneration = _cache.generation;
    final data = await fetchCollectionPageGroups(collectionType, groupTitles);
    if (generation == _generation &&
        cacheGeneration == _cache.generation &&
        !_disposed) {
      _cache.cacheGroups(collectionType, groupKey, data);
    }
    return data;
  }

  Future<void> fetchData() {
    if (_disposed) return Future.value();
    // Appending while a reload runs, or to the items a failed reload left,
    // would mix old and new pages, so the reload fetches the page instead.
    if (_reloading != null || _needsFullReload) {
      _pageRequested = true;
      pageError = null;
      return _reloading ??= _reload(_generation);
    }
    final active = _pageRequest;
    if (active != null) return active;
    if (isLastPage) return Future.value();
    initialized = true;
    pageError = null;
    final request = _loadPage(_generation, cursor);
    _pageRequest = request;
    notifyListeners();
    return request;
  }

  Future<void> _loadPage(int generation, int page) async {
    try {
      final (newItems, newIsLastPage) = await _fetchPage(page);
      if (_disposed || generation != _generation) return;
      items.addAll(newItems);
      cursor = page + 1;
      isLastPage = newIsLastPage;
    } catch (error) {
      if (!_disposed && generation == _generation) pageError = error;
    } finally {
      if (!_disposed && generation == _generation) {
        _pageRequest = null;
        notifyListeners();
      }
    }
  }

  // The reload passes [reloadData] runs, if any. Calls that come while one is
  // out make one more pass after it, not one each.
  Future<void>? _reloadPasses;
  bool _reloadAgain = false;

  /// Loads the list again, keeping the items on screen until the new ones
  /// have arrived. A failure keeps the old items and sets [pageError], and the
  /// returned future completes either way.
  Future<void> reloadData() {
    final running = _reloadPasses;
    if (running != null) {
      _reloadAgain = true;
      return running;
    }
    return _reloadPasses = _runReloadPasses();
  }

  Future<void> _runReloadPasses() async {
    try {
      do {
        _reloadAgain = false;
        await _startReload();
      } while (_reloadAgain && !_disposed);
    } finally {
      _reloadPasses = null;
    }
  }

  Future<void> _startReload() {
    if (_disposed) return Future.value();
    _generation++;
    // The new generation drops the append in flight; the reload fetches its
    // page instead.
    if (_pageRequest != null) _pageRequested = true;
    _pageRequest = null;
    _cache.clearType(collectionType);
    summary = _fetchSummaryWithCache();
    pageError = null;
    notifyListeners();
    return _reloading = _reload(_generation);
  }

  // Keeps the old items on screen until the fresh pages have all arrived,
  // so a scrolled list does not jump back to the start. If a page fails,
  // the old items stay, [pageError] tells the screens, as a failed page does,
  // and the next request reloads from page zero.
  Future<void> _reload(int generation) async {
    final fresh = <Group<InternalCollection>>[];
    var fetched = 0;
    var last = false;
    try {
      while (!last && fetched < max(cursor + (_pageRequested ? 1 : 0), 1)) {
        final (newItems, newIsLastPage) = await _fetchPage(fetched);
        if (_disposed || generation != _generation) return;
        fresh.addAll(newItems);
        fetched += 1;
        last = newIsLastPage;
      }
      items = fresh;
      cursor = fetched;
      isLastPage = last;
      initialized = true;
      _reloading = null;
      _needsFullReload = false;
      _pageRequested = false;
      notifyListeners();
    } catch (error) {
      // A newer reload owns the list now, or the provider is gone.
      if (_disposed || generation != _generation) return;
      _reloading = null;
      _needsFullReload = true;
      _pageRequested = false;
      pageError = error;
      notifyListeners();
    }
  }

  static void clearAllCache() {
    CollectionCache().clearAll();
  }
}
